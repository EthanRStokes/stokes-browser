use ash::vk::Handle;
/// Shared-Vulkan-image helpers for zero-copy cross-process rendering.
///
/// # Platform support
/// * **Windows** – uses `VK_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_WIN32_BIT` + `DuplicateHandle`.
/// * **Linux**   – uses `VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT` (via `VK_EXT_external_memory_dma_buf`).
///
/// # How it works
///
/// The tab (child) process:
///  1. Calls `TabVkImage::new` to allocate a `VkImage` backed by exportable memory.
///  2. Creates a Skia GPU surface that renders into the image.
///  3. After each rendered frame, calls `export_handle` which returns a `u64` that
///     can be sent directly in the `FrameRendered` IPC message.
///     * Windows: a duplicated Win32 HANDLE already valid in the parent process.
///     * Linux:   a dup'd file descriptor number (the parent must `close()` it).
///
/// The parent process:
///  1. Receives the handle value from `FrameRendered`.
///  2. Calls `import_vk_image_raw` to bind the memory into its own Vulkan device and
///     produce an `ImportedVkImage` that can be blitted directly to the swapchain.

use skia_safe::gpu::vk::AllocFlag;
use skia_safe::gpu::{self, DirectContext};
use std::ffi::c_char;
use std::fs::File;
#[cfg(not(windows))]
use std::os::fd::{FromRawFd, IntoRawFd};
#[cfg(windows)]
use std::os::windows::io::IntoRawHandle;
use std::sync::Arc;
use vulkano::device::{Device, Queue};
use vulkano::device::physical::PhysicalDevice;
use vulkano::format::Format;
use vulkano::image::sys::RawImage;
use vulkano::image::{Image, ImageCreateInfo, ImageLayout, ImageSubresourceRange, ImageTiling, ImageType, ImageUsage, SampleCount};
use vulkano::instance::Instance;
use vulkano::memory::allocator::MemoryAllocator;
use vulkano::memory::{
    DedicatedAllocation, DeviceMemory, ExternalMemoryHandleType, MemoryAllocateInfo, MemoryImportInfo,
    MemoryPropertyFlags, MemoryRequirements, ResourceMemory,
};
use vulkano::sync::Sharing;
use vulkano::VulkanObject;
use vulkano::command_buffer::{CommandBufferBeginInfo, CommandBufferLevel, CommandBufferUsage, RecordingCommandBuffer};
use vulkano::command_buffer::allocator::CommandBufferAllocator;
use vulkano::command_buffer::pool::{CommandBufferAllocateInfo, CommandPool, CommandPoolCreateFlags, CommandPoolCreateInfo};
use vulkano::sync::fence::{Fence, FenceCreateInfo};
// ── VkFormat ↔ Skia mappings ────────────────────────────────────────────────

/// Map a `vk::Format` to the corresponding Skia format + color type.
/// Returns `None` for formats Skia cannot handle directly.
pub fn ash_format_to_skia(fmt: ash::vk::Format) -> Option<(gpu::vk::Format, skia_safe::ColorType)> {
    match fmt {
        ash::vk::Format::B8G8R8A8_UNORM => Some((gpu::vk::Format::B8G8R8A8_UNORM, skia_safe::ColorType::BGRA8888)),
        ash::vk::Format::R8G8B8A8_UNORM => Some((gpu::vk::Format::R8G8B8A8_UNORM, skia_safe::ColorType::RGBA8888)),
        ash::vk::Format::B8G8R8A8_SRGB  => Some((gpu::vk::Format::B8G8R8A8_SRGB,  skia_safe::ColorType::BGRA8888)),
        ash::vk::Format::R8G8B8A8_SRGB  => Some((gpu::vk::Format::R8G8B8A8_SRGB,  skia_safe::ColorType::RGBA8888)),
        _ => None,
    }
}

pub fn vk_format_to_skia(fmt: Format) -> Option<(gpu::vk::Format, skia_safe::ColorType)> {
    match fmt {
        Format::B8G8R8A8_UNORM => Some((gpu::vk::Format::B8G8R8A8_UNORM, skia_safe::ColorType::BGRA8888)),
        Format::R8G8B8A8_UNORM => Some((gpu::vk::Format::R8G8B8A8_UNORM, skia_safe::ColorType::RGBA8888)),
        Format::B8G8R8A8_SRGB  => Some((gpu::vk::Format::B8G8R8A8_SRGB,  skia_safe::ColorType::BGRA8888)),
        Format::R8G8B8A8_SRGB  => Some((gpu::vk::Format::R8G8B8A8_SRGB,  skia_safe::ColorType::RGBA8888)),
        _ => None,
    }
}

// ── Platform-specific external memory handle type ─────────────────────────

#[cfg(windows)]
fn external_handle_type() -> ExternalMemoryHandleType {
    ExternalMemoryHandleType::OpaqueWin32
}

#[cfg(not(windows))]
fn external_handle_type() -> ExternalMemoryHandleType {
    ExternalMemoryHandleType::DmaBuf
}

// ── Shared Skia proc-loader ─────────────────────────────────────────────────

/// Helper struct that resolves Vulkan function pointers for Skia.
///
/// The same logic is needed by both the parent (window.rs) and child
/// (tab_process.rs) Vulkan initialisation paths.
pub struct SkiaGetProc {
    get_instance_proc_addr: ash::vk::PFN_vkGetInstanceProcAddr,
    get_device_proc_addr: ash::vk::PFN_vkGetDeviceProcAddr,
}

impl SkiaGetProc {
    /// Build from vulkano instance + device.
    pub fn new(instance: &Arc<Instance>, _device: &Arc<Device>) -> Self {
        let entry = unsafe { ash::Entry::load().expect("Failed to load Vulkan entry for Skia proc resolution") };
        let ash_instance = unsafe { ash::Instance::load(entry.static_fn(), instance.handle()) };

        Self {
            get_instance_proc_addr: entry.static_fn().get_instance_proc_addr,
            get_device_proc_addr: ash_instance.fp_v1_0().get_device_proc_addr,
        }
    }

    /// Resolve a Vulkan proc for Skia.
    pub fn resolve(&self, of: gpu::vk::GetProcOf) -> *const core::ffi::c_void {
        unsafe {
            match of {
                gpu::vk::GetProcOf::Instance(raw_instance, name) => {
                    let vk_instance = if raw_instance.addr() == 0 {
                        ash::vk::Instance::null()
                    } else {
                        ash::vk::Instance::from_raw(raw_instance as _)
                    };
                    (self.get_instance_proc_addr)(vk_instance, name)
                        .map(|f| f as *const core::ffi::c_void)
                        .unwrap_or(std::ptr::null())
                }
                gpu::vk::GetProcOf::Device(raw_device, name) => {
                    if raw_device.addr() == 0 {
                        (self.get_instance_proc_addr)(ash::vk::Instance::null(), name)
                            .map(|f| f as *const core::ffi::c_void)
                            .unwrap_or(std::ptr::null())
                    } else {
                        let vk_device = ash::vk::Device::from_raw(raw_device as _);
                        (self.get_device_proc_addr)(vk_device, name)
                            .map(|f| f as *const core::ffi::c_void)
                            .unwrap_or(std::ptr::null())
                    }
                }
            }
        }
    }
}

pub fn raw_instance_handle(instance: &Arc<Instance>) -> usize {
    instance.handle().as_raw() as usize
}

pub fn raw_physical_device_handle(physical_device: &Arc<PhysicalDevice>) -> usize {
    physical_device.handle().as_raw() as usize
}

pub fn raw_device_handle(device: &Arc<Device>) -> usize {
    device.handle().as_raw() as usize
}

pub fn raw_queue_handle(queue: &Arc<Queue>) -> usize {
    queue.handle().as_raw() as usize
}

pub fn raw_image_handle(image: &Arc<Image>) -> usize {
    image.handle().as_raw() as usize
}

pub fn negotiated_api_version(instance: &Arc<Instance>, physical_device: &Arc<PhysicalDevice>) -> u32 {
    let version = std::cmp::min(instance.api_version(), physical_device.api_version());
    ash::vk::make_api_version(0, version.major, version.minor, version.patch)
}

pub fn color_subresource_range() -> ImageSubresourceRange {
    ImageSubresourceRange {
        aspects: vulkano::image::ImageAspects::COLOR,
        mip_levels: 0..1,
        array_layers: 0..1,
    }
}

pub fn raw_vk_format_to_vulkano(raw: i32) -> Option<Format> {
    match raw {
        x if x == Format::B8G8R8A8_UNORM as i32 => Some(Format::B8G8R8A8_UNORM),
        x if x == Format::R8G8B8A8_UNORM as i32 => Some(Format::R8G8B8A8_UNORM),
        x if x == Format::B8G8R8A8_SRGB as i32 => Some(Format::B8G8R8A8_SRGB),
        x if x == Format::R8G8B8A8_SRGB as i32 => Some(Format::R8G8B8A8_SRGB),
        _ => None,
    }
}

// ── Common color subresource range ──────────────────────────────────────────

/// Standard color subresource range used for layout transitions.
pub const COLOR_SUBRESOURCE_RANGE: ash::vk::ImageSubresourceRange = ash::vk::ImageSubresourceRange {
    aspect_mask: ash::vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

// ── TabVkImage (child/tab side) ─────────────────────────────────────────────

/// A Vulkan image + device-memory pair owned by the tab process.
/// Memory is allocated with the appropriate exportable handle type for the
/// current platform so the parent can import it.
pub struct TabVkImage {
    device: Arc<Device>,
    pub image: Arc<Image>,
    pub memory: Arc<DeviceMemory>,
    pub width: u32,
    pub height: u32,
    pub format: Format,
    /// Exact allocation size (bytes) — must be sent to the parent for import.
    pub alloc_size: u64,
    skia_surface: Option<skia_safe::Surface>,
    pub queue: Arc<Queue>,
}

impl TabVkImage {
    /// Create a new exportable Vulkan image and a Skia GPU surface backed by it.
    ///
    /// `gr_context` must have been created against the same Vulkan device.
    pub unsafe fn new(
        _instance: Arc<Instance>,
        physical_device: Arc<PhysicalDevice>,
        device: Arc<Device>,
        _memory_allocator: Arc<dyn MemoryAllocator>,
        gr_context: &mut DirectContext,
        width: u32,
        height: u32,
        format: Format,
        _queue_family_index: u32,
        queue: Arc<Queue>,
    ) -> Result<Self, String> {
        let (skia_format, color_type) = vk_format_to_skia(format)
            .ok_or_else(|| format!("Unsupported format {:?}", format))?;

        let handle_type = external_handle_type();

        let create_info = ImageCreateInfo {
            flags: Default::default(),
            image_type: ImageType::Dim2d,
            format,
            view_formats: vec![],
            extent: [width, height, 1],
            array_layers: 1,
            mip_levels: 1,
            samples: SampleCount::Sample1,
            tiling: ImageTiling::Optimal,
            usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_SRC,
            stencil_usage: None,
            sharing: Sharing::Exclusive,
            initial_layout: ImageLayout::Undefined,
            drm_format_modifiers: vec![],
            drm_format_modifier_plane_layouts: vec![],
            // Critical: declare that this image will be backed by exportable external memory.
            external_memory_handle_types: handle_type.into(),
            ..Default::default()
        };

        // Vulkano panics if we construct an allocator-backed `Image` but then try to
        // manually allocate and bind memory. For external-memory export/import we
        // keep the whole path manual: RawImage + DeviceMemory + bind.
        let raw_image = RawImage::new(device.clone(), create_info.clone())
            .map_err(|e| format!("Failed to create raw image: {:?}", e))?;

        let mem_reqs_list = raw_image.memory_requirements();
        let mem_reqs = mem_reqs_list
            .last()
            .ok_or_else(|| "Vulkan returned no memory requirements for image".to_string())?;
        let mem_req_size = mem_reqs.layout.size();

        let mem_type_index = find_memory_type(
            physical_device.clone(),
            mem_reqs.memory_type_bits,
            MemoryPropertyFlags::DEVICE_LOCAL,
        )
        .or_else(|| {
            // Some drivers won't expose DEVICE_LOCAL for dma-buf-compatible memory.
            find_memory_type(physical_device.clone(), mem_reqs.memory_type_bits, MemoryPropertyFlags::empty())
        })
        .ok_or_else(|| {
            format!(
                "No suitable memory type for image allocation (bits=0x{:x})",
                mem_reqs.memory_type_bits
            )
        })?;

        let alloc_info = MemoryAllocateInfo {
            allocation_size: mem_reqs.layout.size(),
            memory_type_index: mem_type_index,
            dedicated_allocation: Some(DedicatedAllocation::Image(&raw_image)),
            export_handle_types: handle_type.into(),
            ..Default::default()
        };

        let memory = Arc::new(
            DeviceMemory::allocate(device.clone(), alloc_info)
                .map_err(|e| format!("Failed to allocate exportable image memory: {:?}", e))?,
        );

        let resource_memory = ResourceMemory::new_dedicated_unchecked(memory.clone());

        let image = raw_image
            .bind_memory([resource_memory])
            .map_err(|(e, img, iter)| {
                // Ensure we don't leak the image handle on failure.
                drop(img);
                format!("Failed to bind image memory: {:?}", e)
            })?;

        let alloc = gpu::vk::Alloc::from_device_memory(
            memory.handle().as_raw() as _,
            0,
            mem_req_size,
            AllocFlag::empty(),
        );

        let sk_image_info = gpu::vk::ImageInfo::new(
            image.handle().as_raw() as _,
            alloc,
            gpu::vk::ImageTiling::OPTIMAL,
            gpu::vk::ImageLayout::UNDEFINED,
            skia_format,
            1,
            None,
            None,
            None,
            None,
        );

        let render_target = gpu::backend_render_targets::make_vk((width as i32, height as i32), &sk_image_info);

        let skia_surface = gpu::surfaces::wrap_backend_render_target(
            gr_context,
            &render_target,
            gpu::SurfaceOrigin::TopLeft,
            color_type,
            None,
            None,
        )
        .ok_or_else(|| "Failed to wrap Vulkan image as Skia surface".to_string())?;

        Ok(Self {
            device: device.clone(),
            image: Arc::new(image),
            memory,
            width,
            height,
            format,
            alloc_size: mem_req_size,
            skia_surface: Some(skia_surface),
            queue,
        })
    }

    /// Get a mutable reference to the Skia surface for rendering.
    pub fn surface_mut(&mut self) -> &mut skia_safe::Surface {
        self.skia_surface.as_mut().expect("Skia surface not initialised")
    }

    /// Export the backing memory as a platform handle that can be sent over IPC.
    ///
    /// **Windows**: returns a Win32 `HANDLE` that has already been duplicated into the
    ///   target process (`parent_pid`). The parent process can use this value directly.
    ///
    /// **Linux**: returns a dup'd file-descriptor number (i64 cast to u64).
    pub unsafe fn export_handle(&self, _parent_pid: u32) -> Result<u64, String> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::{
                CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE, INVALID_HANDLE_VALUE,
            };
            use windows_sys::Win32::System::Threading::{
                GetCurrentProcess, OpenProcess, PROCESS_DUP_HANDLE,
            };

            let get_info = ash::vk::MemoryGetWin32HandleInfoKHR::default()
                .memory(self.memory.handle())
                .handle_type(ash::vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32);

            let mut local_handle: ash::vk::HANDLE = 0;
            let res = (self.device.fns().khr_external_memory_win32.get_memory_win32_handle_khr)(
                self.device.handle(),
                &get_info,
                &mut local_handle,
            );
            if res != ash::vk::Result::SUCCESS {
                return Err(format!("vkGetMemoryWin32HandleKHR failed: {:?}", res));
            }

            let parent_proc: HANDLE = OpenProcess(PROCESS_DUP_HANDLE, 0, parent_pid);
            if parent_proc == 0 as HANDLE || parent_proc == INVALID_HANDLE_VALUE {
                let err = std::io::Error::last_os_error();
                CloseHandle(local_handle as HANDLE);
                return Err(format!("OpenProcess({}) failed: {}", parent_pid, err));
            }

            let mut dup_handle: HANDLE = 0 as HANDLE;
            let ok = DuplicateHandle(
                GetCurrentProcess(),
                local_handle as HANDLE,
                parent_proc,
                &mut dup_handle,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            );
            CloseHandle(parent_proc);
            CloseHandle(local_handle as HANDLE);

            if ok == 0 {
                let err = std::io::Error::last_os_error();
                return Err(format!("DuplicateHandle failed: {}", err));
            }

            Ok(dup_handle as u64)
        }

        #[cfg(not(windows))]
        {
            self.memory
                .export_fd(ExternalMemoryHandleType::DmaBuf)
                .map(|file| file.into_raw_fd() as u64)
                .map_err(|e| format!("Failed to export memory fd: {:?}", e))
        }
    }
}

impl Drop for TabVkImage {
    fn drop(&mut self) {
        self.skia_surface = None;

        unsafe {
            self.device.wait_idle().ok();
        }
    }
}

// ── ImportedVkImage: RAII wrapper for resources imported by the parent ──────

/// Owns a VkImage + VkDeviceMemory imported from a tab process.
/// Cleaned up when dropped.
pub struct ImportedVkImage {
    device: Arc<Device>,
    image: Arc<Image>,
    memory: Arc<DeviceMemory>,
}

impl ImportedVkImage {
    /// The raw `VkImage` handle, needed for pipeline barriers.
    #[inline]
    pub fn image(&self) -> Arc<Image> {
        self.image.clone()
    }
}

// ── Parent-side import ───────────────────────────────────────────────────────

/// Import a cross-process memory handle into the parent's Vulkan device as an
/// `ImportedVkImage`. The frame will be composited via `vkCmdBlitImage`.
///
/// # Safety
/// The caller must not use `handle` after this call; ownership is transferred to
/// the Vulkan driver.
pub unsafe fn import_vk_image_raw(
    physical_device: Arc<PhysicalDevice>,
    device: Arc<Device>,
    handle: u64,
    width: u32,
    height: u32,
    format: Format,
    alloc_size: u64,
) -> Result<ImportedVkImage, String> {
    let handle_type = external_handle_type();

    let image_ci = ImageCreateInfo {
        image_type: ImageType::Dim2d,
        format,
        extent: [width, height, 1],
        array_layers: 1,
        mip_levels: 1,
        samples: SampleCount::Sample1,
        tiling: ImageTiling::Optimal,
        usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_SRC,
        sharing: Sharing::Exclusive,
        initial_layout: ImageLayout::Undefined,
        external_memory_handle_types: handle_type.into(),
        ..Default::default()
    };

    // For imported external memory we must create an unbound raw image, import
    // memory from the received handle, then bind that memory to this image.
    let raw_image = RawImage::new(device.clone(), image_ci)
        .map_err(|e| format!("Failed to create raw image for import: {:?}", e))?;

    let mem_reqs = raw_image.memory_requirements();
    let required_alloc_size = mem_reqs
        .last()
        .ok_or_else(|| "Vulkan returned no memory requirements for imported image".to_string())?
        .layout
        .size();

    if alloc_size != required_alloc_size {
        return Err(format!(
            "Imported handle allocation size mismatch: exported={}, import-required={}",
            alloc_size, required_alloc_size
        ));
    }

    let imported_memory = import_memory(
        physical_device,
        device.clone(),
        &raw_image,
        handle,
        required_alloc_size,
        Vec::from(mem_reqs),
    )?;

    let imported_memory = Arc::new(imported_memory);
    let resource_memory = ResourceMemory::new_dedicated_unchecked(imported_memory.clone());
    let image = raw_image
        .bind_memory([resource_memory])
        .map_err(|(e, img, _)| {
            drop(img);
            format!("Failed to bind imported memory to image: {:?}", e)
        })?;

    Ok(ImportedVkImage {
        device,
        image: Arc::new(image),
        memory: imported_memory,
    })
}

// ── Platform-specific memory import helpers ─────────────────────────────────

#[cfg(windows)]
unsafe fn import_memory(
    physical_device: Arc<PhysicalDevice>,
    device: Arc<Device>,
    raw_image: &RawImage,
    handle: u64,
    alloc_size: u64,
    mem_reqs: Vec<MemoryRequirements>,
) -> Result<DeviceMemory, String> {
    let mem_reqs = mem_reqs
        .last()
        .ok_or_else(|| "Vulkan returned no memory requirements for imported image".to_string())?;

    let mem_type_index = find_memory_type(
        physical_device.clone(),
        mem_reqs.memory_type_bits,
        MemoryPropertyFlags::DEVICE_LOCAL,
    )
    .or_else(|| {
        find_memory_type(
            physical_device.clone(),
            mem_reqs.memory_type_bits,
            MemoryPropertyFlags::empty(),
        )
    })
    .ok_or_else(|| {
        format!(
            "No compatible memory type for import (win32): bits=0x{:x}",
            mem_reqs.memory_type_bits
        )
    })?;

    let import_info = MemoryImportInfo::Win32 {
        handle_type: ExternalMemoryHandleType::OpaqueWin32,
        handle: handle as ash::vk::HANDLE,
    };

    let alloc_info = MemoryAllocateInfo {
        allocation_size: alloc_size,
        memory_type_index: mem_type_index,
        // Required by VUID-VkBindImageMemoryInfo-image-01445 when the image
        // reports requires_dedicated_allocation.
        dedicated_allocation: Some(DedicatedAllocation::Image(raw_image)),
        ..Default::default()
    };

    DeviceMemory::import(device, alloc_info, import_info)
        .map_err(|e| format!("vkAllocateMemory (win32 import) failed: {:?}", e))
}

#[cfg(not(windows))]
unsafe fn import_memory(
    physical_device: Arc<PhysicalDevice>,
    device: Arc<Device>,
    raw_image: &RawImage,
    handle: u64,
    alloc_size: u64,
    mem_reqs: Vec<MemoryRequirements>,
) -> Result<DeviceMemory, String> {
    let fd = unsafe { File::from_raw_fd(handle as i32) };
    let mem_reqs = mem_reqs
        .last()
        .ok_or_else(|| "Vulkan returned no memory requirements for imported image".to_string())?;

    let fd_props = device
        .memory_fd_properties(
            ExternalMemoryHandleType::DmaBuf,
            fd.try_clone().expect("Failed to clone fd"),
        )
        .map_err(|e| format!("vkGetMemoryFdPropertiesKHR failed: {:?}", e))?;

    let combined_bits = mem_reqs.memory_type_bits & fd_props.memory_type_bits;
    let candidate_bits = if combined_bits != 0 { combined_bits } else { fd_props.memory_type_bits };

    let mem_type_index = find_memory_type(
        physical_device.clone(),
        candidate_bits,
        MemoryPropertyFlags::DEVICE_LOCAL,
    )
    .or_else(|| {
        find_memory_type(physical_device.clone(), candidate_bits, MemoryPropertyFlags::empty())
    })
    .ok_or_else(|| {
        format!(
            "No compatible memory type for import (dma-buf fd): \
             image bits=0x{:x}, fd bits=0x{:x}",
            mem_reqs.memory_type_bits, fd_props.memory_type_bits
        )
    })?;

    let import_info = MemoryImportInfo::Fd {
        handle_type: ExternalMemoryHandleType::DmaBuf,
        file: fd,
    };

    let alloc_info = MemoryAllocateInfo {
        allocation_size: alloc_size,
        memory_type_index: mem_type_index,
        // Required by VUID-VkBindImageMemoryInfo-image-01445 when the image
        // reports requires_dedicated_allocation.
        dedicated_allocation: Some(DedicatedAllocation::Image(raw_image)),
        ..Default::default()
    };

    DeviceMemory::import(device, alloc_info, import_info)
        .map_err(|e| format!("vkAllocateMemory (dma-buf fd import) failed: {:?}", e))
}

// ── Helper: find_memory_type ────────────────────────────────────────────────

pub fn find_memory_type(
    physical_device: Arc<PhysicalDevice>,
    type_filter: u32,
    properties: MemoryPropertyFlags,
) -> Option<u32> {
    let mem_props = physical_device.memory_properties();
    let mem_types = &mem_props.memory_types;
    for (i, ty) in mem_types.iter().enumerate() {
        if (type_filter & (1 << i)) != 0 && ty.property_flags.contains(properties) {
            return Some(i as u32);
        }
    }
    None
}

// ── Device extension lists ──────────────────────────────────────────────────

/// External-memory extensions shared by parent and tab.
#[cfg(windows)]
fn shared_external_device_extensions() -> Vec<*const c_char> {
    vec![
        ash::khr::external_memory::NAME.as_ptr(),
        ash::khr::external_memory_win32::NAME.as_ptr(),
    ]
}

/// External-memory extensions shared by parent and tab.
#[cfg(not(windows))]
fn shared_external_device_extensions() -> Vec<*const c_char> {
    vec![
        ash::khr::external_memory::NAME.as_ptr(),
        ash::khr::external_memory_fd::NAME.as_ptr(),
        ash::vk::EXT_EXTERNAL_MEMORY_DMA_BUF_NAME.as_ptr(),
    ]
}

/// The minimal Vulkan info the parent passes to each tab process so they can
/// connect to the same physical device.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VulkanDeviceInfo {
    /// VkPhysicalDeviceIDProperties::deviceUUID — stable across processes.
    pub device_uuid: [u8; 16],
    /// Queue family index used by the parent.
    pub queue_family_index: u32,
    /// Swapchain image format (as raw `VkFormat` integer).
    pub image_format: i32,
    /// PID of the parent process (needed on Windows for `DuplicateHandle`).
    pub parent_pid: u32,
}

/// Query the `deviceUUID` for a physical device (requires Vulkan 1.1+).
pub fn physical_device_uuid(physical_device: &Arc<PhysicalDevice>) -> [u8; 16] {
    physical_device.properties().device_uuid.unwrap_or_default()
}


/// Device extension names for the browser (parent) Vulkan device.
pub fn parent_device_extension_names() -> Vec<*const c_char> {
    let mut exts = vec![ash::khr::swapchain::NAME.as_ptr()];
    exts.extend(shared_external_device_extensions());
    exts
}

/// Device extension names for the tab-process Vulkan device.
pub fn tab_device_extension_names() -> Vec<*const c_char> {
    shared_external_device_extensions()
}

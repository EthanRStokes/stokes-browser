use ash::vk::{Handle, HANDLE};
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
use std::os::fd::IntoRawFd;
use std::sync::Arc;
use vulkano::device::{Device, Queue};
use vulkano::format::Format;
use vulkano::image::{Image, ImageCreateInfo, ImageLayout, ImageTiling, ImageType, ImageUsage, SampleCount};
use vulkano::instance::Instance;
use vulkano::memory::allocator::{AllocationCreateInfo, AllocationType, MemoryAllocator};
use vulkano::memory::{DedicatedAllocation, DeviceMemory, ExternalMemoryHandleType, MemoryAllocateInfo, MemoryImportInfo, MemoryPropertyFlags, MemoryRequirements, ResourceMemory};
use vulkano::sync::semaphore::{ExternalSemaphoreHandleType, ExternalSemaphoreHandleTypes, Semaphore, SemaphoreCreateInfo};
use vulkano::sync::{DependencyInfo, ImageMemoryBarrier, Sharing};
use vulkano::{Validated, VulkanError, VulkanObject};
use vulkano::command_buffer::{CommandBuffer, CommandBufferBeginInfo, CommandBufferLevel, CommandBufferUsage, RecordingCommandBuffer};
use vulkano::command_buffer::allocator::CommandBufferAllocator;
use vulkano::command_buffer::pool::{CommandBufferAllocateInfo, CommandPool, CommandPoolAlloc, CommandPoolCreateFlags, CommandPoolCreateInfo};
use vulkano::device::physical::PhysicalDevice;
use vulkano::image::sys::RawImage;
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

// ── Platform-specific external semaphore handle type ──────────────────────

#[cfg(windows)]
fn external_semaphore_handle_type() -> ExternalSemaphoreHandleType {
    ExternalSemaphoreHandleType::OpaqueWin32
}

#[cfg(not(windows))]
fn external_semaphore_handle_type() -> ExternalSemaphoreHandleType {
    ExternalSemaphoreHandleType::SyncFd
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
    /// Build from ash Entry + Instance.
    pub fn new(entry: &ash::Entry, instance: &ash::Instance) -> Self {
        Self {
            get_instance_proc_addr: entry.static_fn().get_instance_proc_addr,
            get_device_proc_addr: instance.fp_v1_0().get_device_proc_addr,
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
    /// Exportable semaphore signaled after each render to synchronize the parent.
    pub render_semaphore: Option<TabVkSemaphore>,
    /// Command pool for the layout-transition submit done after each render.
    cmd_pool: Arc<CommandPool>,
    /// Pre-recorded command buffer: COLOR_ATTACHMENT_OPTIMAL → GENERAL barrier.
    cmd_buf: Arc<CommandPoolAlloc>,
    cmd_buf_allocator: Arc<dyn CommandBufferAllocator>,
    pub queue: Arc<Queue>,
    queue_family_index: u32,
    /// Fence used to track the last `signal_and_export_semaphore` submission.
    submit_fence: Arc<Fence>,
    /// Whether `submit_fence` is currently in the submitted (unsignaled) state.
    submit_fence_pending: bool,

    // todo check
    //#[cfg(windows)]
    //ext_mem_win32: ash::khr::external_memory_win32::Device,
    //#[cfg(not(windows))]
    //ext_mem_fd: ash::khr::external_memory_fd::Device,
}

impl TabVkImage {
    /// Create a new exportable Vulkan image and a Skia GPU surface backed by it.
    ///
    /// `gr_context` must have been created against the same Vulkan device.
    pub unsafe fn new(
        instance: Arc<Instance>,
        physical_device: Arc<PhysicalDevice>,
        device: Arc<Device>,
        memory_allocator: Arc<dyn MemoryAllocator>,
        cmd_buf_allocator: Arc<dyn CommandBufferAllocator>,
        gr_context: &mut DirectContext,
        width: u32,
        height: u32,
        format: Format,
        queue_family_index: u32,
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
            usage: ImageUsage::COLOR_ATTACHMENT
                | ImageUsage::SAMPLED
                | ImageUsage::TRANSFER_SRC
                | ImageUsage::TRANSFER_DST,
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

        // -- Build Skia surface -----------------------------------------------
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

        let pool_ci = CommandPoolCreateInfo {
            flags: CommandPoolCreateFlags::RESET_COMMAND_BUFFER,
            queue_family_index,
            ..Default::default()
        };
        let cmd_pool = CommandPool::new(device.clone(), pool_ci).expect("Failed to create command pool");

        let cmd_buf = {
            let ai = CommandBufferAllocateInfo {
                level: CommandBufferLevel::Primary,
                command_buffer_count: 1,
                ..Default::default()
            };
            let bufs = cmd_pool.allocate_command_buffers(ai).expect("Failed to allocate command buffers");
            bufs.into_iter().next().unwrap() // works bc the count is 1
        };

         // -- Exportable render-complete semaphore -----------------------------
         let render_semaphore = TabVkSemaphore::new(instance.clone(), device.clone())
             .ok();

         // -- Fence for tracking signal_and_export_semaphore submissions ------
        let submit_fence = Arc::new(Fence::new(device.clone(), FenceCreateInfo::default())
            .expect("Failed to create Fence"));

        Ok(Self {
            device: device.clone(),
            image: Arc::new(image),
            memory,
            width,
            height,
            format,
            alloc_size: mem_req_size,
            skia_surface: Some(skia_surface),
            render_semaphore,
            cmd_pool: Arc::new(cmd_pool),
            cmd_buf: Arc::new(cmd_buf),
            cmd_buf_allocator,
            queue,
            queue_family_index,
            submit_fence,
            submit_fence_pending: false,
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
    pub unsafe fn export_handle(&self, parent_pid: u32) -> Result<u64, String> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::{
                CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE, INVALID_HANDLE_VALUE,
            };
            use windows_sys::Win32::System::Threading::{
                GetCurrentProcess, OpenProcess, PROCESS_DUP_HANDLE,
            };

            let get_info = ash::vk::MemoryGetWin32HandleInfoKHR::default()
                .memory(self.memory)
                .handle_type(ash::vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32);
            let local_handle = self
                .ext_mem_win32
                .get_memory_win32_handle(&get_info)
                .map_err(|e| format!("vkGetMemoryWin32HandleKHR failed: {:?}", e))?;

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
            self.memory.export_fd(ExternalMemoryHandleType::DmaBuf)
                .map(|file| file.into_raw_fd() as u64)
                .map_err(|e| format!("Failed to export memory fd: {:?}", e))
        }
    }

    /// Submit a queue command that:
    ///   1. Inserts a pipeline barrier transitioning the image from
    ///      `COLOR_ATTACHMENT_OPTIMAL` → `GENERAL` (readable cross-process).
    ///   2. Signals `render_semaphore`.
    ///
    /// Returns the exported semaphore handle, or -1/0 if unavailable.
    /// Must be called *after* `gr_context.flush_and_submit()`.
    pub unsafe fn signal_and_export_semaphore(&mut self, _parent_pid: u32) -> i64 {
        // Wait for any prior submission to complete before reusing the command
        // buffer and fence.
        if self.submit_fence_pending {
            self.submit_fence.wait(None).expect("Failed to wait for submit fence");
            self.submit_fence_pending = false;
        }

        let begin_info = CommandBufferBeginInfo {
            usage: CommandBufferUsage::OneTimeSubmit,
            ..Default::default()
        };
        let mut rec_cmd_buf = RecordingCommandBuffer::new(self.cmd_buf_allocator.clone(), self.queue_family_index, CommandBufferLevel::Primary, begin_info)
            .expect("Failed to begin recording command buffer");

        // todo check
        let bruh = ash::vk::ImageMemoryBarrier::default()
            .src_access_mask(ash::vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(ash::vk::AccessFlags::empty())
            .old_layout(ash::vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(ash::vk::ImageLayout::GENERAL)
            .src_queue_family_index(self.queue_family_index)
            .dst_queue_family_index(ash::vk::QUEUE_FAMILY_EXTERNAL)
            .image(self.image.handle())
            .subresource_range(COLOR_SUBRESOURCE_RANGE);

        let barrier = DependencyInfo {
            dependency_flags: Default::default(),
            image_memory_barriers: vec![
                ImageMemoryBarrier::image(self.image.clone())
            ].into(),
            ..Default::default()
        };

        rec_cmd_buf.pipeline_barrier(&barrier).expect("Failed to record pipeline barrier");

        rec_cmd_buf.end().expect("Failed to finish recording command buffer");

        let cmd_bufs = [self.cmd_buf.handle()];
        let submit_info = ash::vk::SubmitInfo::default().command_buffers(&cmd_bufs);
        let res = (self.device.fns().v1_0.queue_submit)(self.queue.handle(), 1, &submit_info, self.submit_fence.handle());
        self.submit_fence_pending = true;

        -1
    }
}

impl Drop for TabVkImage {
    fn drop(&mut self) {
        // Drop the Skia surface first — it may hold internal references to
        // the VkImage / VkDeviceMemory through the Skia GPU backend.
        self.skia_surface = None;

        unsafe {
            if self.submit_fence_pending {
                self.submit_fence.wait(None).expect("Failed to wait for submit fence");
                self.submit_fence_pending = false;
            }

            // Also do a full device wait as a safety net — Skia may have
            // submitted additional GPU work we don't track with our fence.
            self.device.wait_idle().ok();

            // Now safe to destroy everything.
            self.render_semaphore = None;
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
    instance: Arc<Instance>,
    physical_device: Arc<PhysicalDevice>,
    device: Arc<Device>,
    allocator: Arc<dyn MemoryAllocator>,
    handle: File,
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
        usage: ImageUsage::TRANSFER_SRC | ImageUsage::TRANSFER_DST | ImageUsage::SAMPLED,
        sharing: Sharing::Exclusive,
        initial_layout: ImageLayout::Undefined,
        external_memory_handle_types: handle_type.into(),
        ..Default::default()
    };

    let image = Image::new(
        allocator.clone(),
        image_ci.clone(),
        Default::default(),
    ).map_err(|e| format!("Failed to create image with external memory: {:?}", e))?;

    let raw_image = RawImage::from_handle_borrowed(device.clone(), image.handle(), image_ci)
        .map_err(|e| {
            format!("Failed to create raw image: {:?}", e)
        })?;

    let mem_reqs = image.memory_requirements();

    let imported_memory =
        import_memory(instance, physical_device, device.clone(), image.clone(), raw_image, handle, alloc_size, mem_reqs)?;

    Ok(ImportedVkImage {
        device: device.clone(),
        image,
        memory: Arc::new(imported_memory),
    })
}

// ── Platform-specific memory import helpers ─────────────────────────────────

#[cfg(windows)]
unsafe fn import_memory(
    instance: &ash::Instance,
    physical_device: ash::vk::PhysicalDevice,
    device: &ash::Device,
    image: ash::vk::Image,
    handle: u64,
    alloc_size: u64,
    mem_reqs: &ash::vk::MemoryRequirements,
) -> Result<ash::vk::DeviceMemory, String> {
    use ash::khr::external_memory_win32;

    let ext = external_memory_win32::Device::new(instance, device);

    let mut handle_props = ash::vk::MemoryWin32HandlePropertiesKHR::default();
    ext.get_memory_win32_handle_properties(
        ash::vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32,
        handle as ash::vk::HANDLE,
        &mut handle_props,
    )
    .map_err(|e| format!("vkGetMemoryWin32HandlePropertiesKHR failed: {:?}", e))?;

    let combined_bits = mem_reqs.memory_type_bits & handle_props.memory_type_bits;
    let mem_type_index = find_memory_type(
        instance,
        physical_device,
        combined_bits,
        ash::vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )
    .ok_or_else(|| {
        unsafe { device.destroy_image(image, None) };
        "No compatible memory type for import (win32)".to_string()
    })?;

    let mut import_info = ash::vk::ImportMemoryWin32HandleInfoKHR::default()
        .handle_type(ash::vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32)
        .handle(handle as ash::vk::HANDLE);

    let mut dedicated_alloc_info = ash::vk::MemoryDedicatedAllocateInfo::default()
        .image(image);

    let alloc_info = ash::vk::MemoryAllocateInfo::default()
        .push_next(&mut import_info)
        .push_next(&mut dedicated_alloc_info)
        .allocation_size(alloc_size)
        .memory_type_index(mem_type_index);

    device
        .allocate_memory(&alloc_info, None)
        .map_err(|e| {
            unsafe { device.destroy_image(image, None) };
            format!("vkAllocateMemory (win32 import) failed: {:?}", e)
        })
}

#[cfg(not(windows))]
unsafe fn import_memory(
    instance: Arc<Instance>,
    physical_device: Arc<PhysicalDevice>,
    device: Arc<Device>,
    image: Arc<Image>,
    raw_image: RawImage,
    handle: File,
    alloc_size: u64,
    mem_reqs: &[MemoryRequirements],
) -> Result<DeviceMemory, String> {
    let fd = handle;
    // todo check
    let mem_reqs = mem_reqs.last().unwrap();

    // todo check
    //let ext_fd = ash::khr::external_memory_fd::Device::new(instance, device);

    let mut fd_props = ash::vk::MemoryFdPropertiesKHR::default();
    device
        .memory_fd_properties(
            ExternalMemoryHandleType::DmaBuf,
            fd.try_clone().expect("Failed to clone fd"),
        )
        .map_err(|e| {
            unsafe { (device.fns().v1_0.destroy_image)(device.handle(), image.handle(), std::ptr::null()) };
            format!("vkGetMemoryFdPropertiesKHR failed: {:?}", e)
        })?;

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
        dedicated_allocation: Some(
            DedicatedAllocation::Image(&raw_image)
        ),
        ..Default::default()
    };

    let image2 = image.clone();
    DeviceMemory::import(device.clone(), alloc_info, import_info)
        .map_err(|e| {
            unsafe { (device.fns().v1_0.destroy_image)(device.handle(), image2.handle(), std::ptr::null()) };
            format!("vkAllocateMemory (dma-buf fd import) failed: {:?}", e)
        })
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

// ── TabVkSemaphore (child/tab side) ─────────────────────────────────────────

/// An exportable binary `VkSemaphore` owned by the tab process.
pub struct TabVkSemaphore {
    device: Arc<Device>,
    pub semaphore: Arc<Semaphore>,
    // todo check
    //#[cfg(windows)]
    //ext_sem_win32: Arc<Device>,
    //#[cfg(not(windows))]
    //ext_sem_fd: Arc<Device>,
}

impl TabVkSemaphore {
    /// Create a new exportable binary semaphore.
    pub unsafe fn new(instance: Arc<Instance>, device: Arc<Device>) -> Result<Self, String> {
        let handle_type = external_semaphore_handle_type();

        let create_info = SemaphoreCreateInfo {
            export_handle_types: ExternalSemaphoreHandleTypes::from(handle_type),
            ..Default::default()
        };

        let semaphore = Semaphore::new(
            device.clone(),
            create_info,
        ).map_err(|e| format!("Failed to create exportable semaphore: {:?}", e))?;

        // todo check
        //#[cfg(windows)]
        //let ext_sem_win32 = ash::khr::external_semaphore_win32::Device::new(instance, device);
        //#[cfg(not(windows))]
        //let ext_sem_fd = Device::new(instance, device);

        Ok(Self {
            device: device.clone(),
            semaphore: Arc::new(semaphore),
            //#[cfg(windows)]
            //ext_sem_win32,
            //#[cfg(not(windows))]
            //ext_sem_fd,
        })
    }

    /// Export the semaphore as a platform handle to send to the parent.
    ///
    /// * Linux   – returns a `sync_fd` (i32 cast to i64).
    /// * Windows – returns a Win32 HANDLE already duplicated into `parent_pid`.
    ///
    /// Returns -1 / 0 on failure (non-fatal; parent falls back to CPU wait).
    pub unsafe fn export(&self, parent_pid: u32) -> i64 {
        let handle_type = external_semaphore_handle_type();

        #[cfg(not(windows))]
        {
            return match self.semaphore.export_fd(handle_type) {
                Ok(file) => file.into_raw_fd() as i64,
                Err(_) => -1
            }
        }

        #[cfg(windows)]
        {
            return match self.semaphore.export_win32_handle(handle_type) {
                Ok(handle) => handle as i64,
                Err(_) => {
                    eprintln!("Failed to export Win32 semaphore handle");
                    0
                }
            }
        }
    }
}

// ── VulkanDeviceInfo: serialisable info the parent passes to child via env ──

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
pub unsafe fn physical_device_uuid(
    instance: &ash::Instance,
    physical_device: ash::vk::PhysicalDevice,
) -> [u8; 16] {
    let mut id_props = ash::vk::PhysicalDeviceIDProperties::default();
    let mut props2 = ash::vk::PhysicalDeviceProperties2::default().push_next(&mut id_props);
    instance.get_physical_device_properties2(physical_device, &mut props2);
    id_props.device_uuid
}

// ── Device extension lists ──────────────────────────────────────────────────

/// External-memory/semaphore extensions shared by parent and tab.
#[cfg(windows)]
fn shared_external_device_extensions() -> Vec<*const c_char> {
    vec![
        ash::khr::external_memory::NAME.as_ptr(),
        ash::khr::external_memory_win32::NAME.as_ptr(),
        ash::khr::external_semaphore::NAME.as_ptr(),
        ash::khr::external_semaphore_win32::NAME.as_ptr(),
    ]
}

/// External-memory/semaphore extensions shared by parent and tab.
#[cfg(not(windows))]
fn shared_external_device_extensions() -> Vec<*const c_char> {
    vec![
        ash::khr::external_memory::NAME.as_ptr(),
        ash::khr::external_memory_fd::NAME.as_ptr(),
        ash::vk::EXT_EXTERNAL_MEMORY_DMA_BUF_NAME.as_ptr(),
        ash::khr::external_semaphore::NAME.as_ptr(),
        ash::khr::external_semaphore_fd::NAME.as_ptr(),
    ]
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

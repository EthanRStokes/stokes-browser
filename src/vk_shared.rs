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

use ash::vk::{self, Handle};
use skia_safe::gpu::vk::AllocFlag;
use skia_safe::gpu::{self, DirectContext};
use std::ffi::c_char;


// ── VkFormat ↔ Skia mappings ────────────────────────────────────────────────

/// Map a `vk::Format` to the corresponding Skia format + color type.
/// Returns `None` for formats Skia cannot handle directly.
pub fn vk_format_to_skia(fmt: vk::Format) -> Option<(gpu::vk::Format, skia_safe::ColorType)> {
    match fmt {
        vk::Format::B8G8R8A8_UNORM => Some((gpu::vk::Format::B8G8R8A8_UNORM, skia_safe::ColorType::BGRA8888)),
        vk::Format::R8G8B8A8_UNORM => Some((gpu::vk::Format::R8G8B8A8_UNORM, skia_safe::ColorType::RGBA8888)),
        vk::Format::B8G8R8A8_SRGB  => Some((gpu::vk::Format::B8G8R8A8_SRGB,  skia_safe::ColorType::BGRA8888)),
        vk::Format::R8G8B8A8_SRGB  => Some((gpu::vk::Format::R8G8B8A8_SRGB,  skia_safe::ColorType::RGBA8888)),
        _ => None,
    }
}

// ── Platform-specific external memory handle type ─────────────────────────

#[cfg(windows)]
fn external_handle_type() -> vk::ExternalMemoryHandleTypeFlags {
    vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32
}

#[cfg(not(windows))]
fn external_handle_type() -> vk::ExternalMemoryHandleTypeFlags {
    vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT
}

// ── Platform-specific external semaphore handle type ──────────────────────

#[cfg(windows)]
fn external_semaphore_handle_type() -> vk::ExternalSemaphoreHandleTypeFlags {
    vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_WIN32
}

#[cfg(not(windows))]
fn external_semaphore_handle_type() -> vk::ExternalSemaphoreHandleTypeFlags {
    vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD
}

// ── Shared Skia proc-loader ─────────────────────────────────────────────────

/// Helper struct that resolves Vulkan function pointers for Skia.
///
/// The same logic is needed by both the parent (window.rs) and child
/// (tab_process.rs) Vulkan initialisation paths.
pub struct SkiaGetProc {
    get_instance_proc_addr: vk::PFN_vkGetInstanceProcAddr,
    get_device_proc_addr: vk::PFN_vkGetDeviceProcAddr,
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
                        vk::Instance::null()
                    } else {
                        vk::Instance::from_raw(raw_instance as _)
                    };
                    (self.get_instance_proc_addr)(vk_instance, name)
                        .map(|f| f as *const core::ffi::c_void)
                        .unwrap_or(std::ptr::null())
                }
                gpu::vk::GetProcOf::Device(raw_device, name) => {
                    if raw_device.addr() == 0 {
                        (self.get_instance_proc_addr)(vk::Instance::null(), name)
                            .map(|f| f as *const core::ffi::c_void)
                            .unwrap_or(std::ptr::null())
                    } else {
                        let vk_device = vk::Device::from_raw(raw_device as _);
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
pub const COLOR_SUBRESOURCE_RANGE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
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
    device: ash::Device,
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
    pub width: u32,
    pub height: u32,
    pub format: vk::Format,
    /// Exact allocation size (bytes) — must be sent to the parent for import.
    pub alloc_size: u64,
    skia_surface: Option<skia_safe::Surface>,
    /// Exportable semaphore signaled after each render to synchronize the parent.
    pub render_semaphore: Option<TabVkSemaphore>,
    /// Command pool for the layout-transition submit done after each render.
    cmd_pool: vk::CommandPool,
    /// Pre-recorded command buffer: COLOR_ATTACHMENT_OPTIMAL → GENERAL barrier.
    cmd_buf: vk::CommandBuffer,
    pub queue: vk::Queue,
    queue_family_index: u32,
    /// Fence used to track the last `signal_and_export_semaphore` submission.
    /// On Windows, `device_wait_idle` alone may not be sufficient to ensure the
    /// driver has fully retired a signaled-but-never-waited-on semaphore; using
    /// a fence gives us a reliable synchronization point.
    submit_fence: vk::Fence,
    /// Whether `submit_fence` is currently in the submitted (unsignaled) state.
    submit_fence_pending: bool,

    #[cfg(windows)]
    ext_mem_win32: ash::khr::external_memory_win32::Device,
    #[cfg(not(windows))]
    ext_mem_fd: ash::khr::external_memory_fd::Device,
}

impl TabVkImage {
    /// Create a new exportable Vulkan image and a Skia GPU surface backed by it.
    ///
    /// `gr_context` must have been created against the same Vulkan device.
    pub unsafe fn new(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        device: &ash::Device,
        gr_context: &mut DirectContext,
        width: u32,
        height: u32,
        format: vk::Format,
        queue_family_index: u32,
        queue: vk::Queue,
    ) -> Result<Self, String> {
        let (skia_format, color_type) = vk_format_to_skia(format)
            .ok_or_else(|| format!("Unsupported format {:?}", format))?;

        let handle_type = external_handle_type();

        // -- Create image with external memory info chained in pNext ----------
        let mut ext_image_ci = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(handle_type);

        let image_ci = vk::ImageCreateInfo::default()
            .push_next(&mut ext_image_ci)
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D { width, height, depth: 1 })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(
                vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::SAMPLED
                    | vk::ImageUsageFlags::TRANSFER_SRC
                    | vk::ImageUsageFlags::TRANSFER_DST,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);

        let image = device
            .create_image(&image_ci, None)
            .map_err(|e| format!("vkCreateImage failed: {:?}", e))?;

        // -- Allocate memory with export info ---------------------------------
        let mem_reqs = device.get_image_memory_requirements(image);

        let mem_type_index = find_memory_type(
            instance,
            physical_device,
            mem_reqs.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )
        .ok_or_else(|| {
            device.destroy_image(image, None);
            "No suitable memory type".to_string()
        })?;

        let mut export_alloc_info = vk::ExportMemoryAllocateInfo::default()
            .handle_types(handle_type);

        let mut dedicated_alloc_info = vk::MemoryDedicatedAllocateInfo::default()
            .image(image);

        let alloc_info = vk::MemoryAllocateInfo::default()
            .push_next(&mut export_alloc_info)
            .push_next(&mut dedicated_alloc_info)
            .allocation_size(mem_reqs.size)
            .memory_type_index(mem_type_index);

        let memory = device.allocate_memory(&alloc_info, None).map_err(|e| {
            device.destroy_image(image, None);
            format!("vkAllocateMemory failed: {:?}", e)
        })?;

        device.bind_image_memory(image, memory, 0).map_err(|e| {
            device.free_memory(memory, None);
            device.destroy_image(image, None);
            format!("vkBindImageMemory failed: {:?}", e)
        })?;

        // -- Build Skia surface -----------------------------------------------
        let alloc = gpu::vk::Alloc::from_device_memory(
            memory.as_raw() as _,
            0,
            mem_reqs.size,
            AllocFlag::empty(),
        );

        let sk_image_info = gpu::vk::ImageInfo::new(
            image.as_raw() as _,
            alloc,
            gpu::vk::ImageTiling::OPTIMAL,
            gpu::vk::ImageLayout::UNDEFINED,
            skia_format,
            1,
            None, None, None, None,
        );

        let render_target = gpu::backend_render_targets::make_vk(
            (width as i32, height as i32),
            &sk_image_info,
        );

        let skia_surface = gpu::surfaces::wrap_backend_render_target(
            gr_context,
            &render_target,
            gpu::SurfaceOrigin::TopLeft,
            color_type,
            None,
            None,
        )
        .ok_or_else(|| {
            unsafe {
                device.free_memory(memory, None);
                device.destroy_image(image, None);
            }
            "Failed to wrap Vulkan image as Skia surface".to_string()
        })?;

        // -- Command pool + buffer for the post-render layout transition ------
        let pool_ci = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        let cmd_pool = device.create_command_pool(&pool_ci, None).map_err(|e| {
            device.free_memory(memory, None);
            device.destroy_image(image, None);
            format!("vkCreateCommandPool (tab) failed: {:?}", e)
        })?;

        let cmd_buf = {
            let ai = vk::CommandBufferAllocateInfo::default()
                .command_pool(cmd_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);
            let bufs = device.allocate_command_buffers(&ai).map_err(|e| {
                device.destroy_command_pool(cmd_pool, None);
                device.free_memory(memory, None);
                device.destroy_image(image, None);
                format!("vkAllocateCommandBuffers (tab) failed: {:?}", e)
            })?;
            bufs[0]
        };

        // -- Exportable render-complete semaphore -----------------------------
        let render_semaphore = TabVkSemaphore::new(instance, device)
            .map_err(|e| {
                device.destroy_command_pool(cmd_pool, None);
                device.free_memory(memory, None);
                device.destroy_image(image, None);
                e
            })
            .ok();

        // -- Fence for tracking signal_and_export_semaphore submissions ------
        let submit_fence = device.create_fence(&vk::FenceCreateInfo::default(), None)
            .map_err(|e| {
                device.destroy_command_pool(cmd_pool, None);
                device.free_memory(memory, None);
                device.destroy_image(image, None);
                format!("vkCreateFence (tab submit) failed: {:?}", e)
            })?;

        #[cfg(windows)]
        let ext_mem_win32 = ash::khr::external_memory_win32::Device::new(instance, device);
        #[cfg(not(windows))]
        let ext_mem_fd = ash::khr::external_memory_fd::Device::new(instance, device);

        Ok(Self {
            device: device.clone(),
            image,
            memory,
            width,
            height,
            format,
            alloc_size: mem_reqs.size,
            skia_surface: Some(skia_surface),
            render_semaphore,
            cmd_pool,
            cmd_buf,
            queue,
            queue_family_index,
            submit_fence,
            submit_fence_pending: false,
            #[cfg(windows)]
            ext_mem_win32,
            #[cfg(not(windows))]
            ext_mem_fd,
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
                CloseHandle, DuplicateHandle, HANDLE, DUPLICATE_SAME_ACCESS, INVALID_HANDLE_VALUE,
            };
            use windows_sys::Win32::System::Threading::{
                GetCurrentProcess, OpenProcess, PROCESS_DUP_HANDLE,
            };

            let get_info = vk::MemoryGetWin32HandleInfoKHR::default()
                .memory(self.memory)
                .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32);
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
            let _ = parent_pid;
            let get_fd_info = vk::MemoryGetFdInfoKHR::default()
                .memory(self.memory)
                .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);

            self.ext_mem_fd
                .get_memory_fd(&get_fd_info)
                .map(|fd| fd as u64)
                .map_err(|e| format!("vkGetMemoryFdKHR failed: {:?}", e))
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
            let _ = self.device.wait_for_fences(
                &[self.submit_fence],
                true,
                5_000_000_000,
            );
            let _ = self.device.reset_fences(&[self.submit_fence]);
            self.submit_fence_pending = false;
        }

        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        if self.device.begin_command_buffer(self.cmd_buf, &begin_info).is_err() {
            return -1;
        }

        let barrier = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::empty())
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_queue_family_index(self.queue_family_index)
            .dst_queue_family_index(vk::QUEUE_FAMILY_EXTERNAL)
            .image(self.image)
            .subresource_range(COLOR_SUBRESOURCE_RANGE);

        self.device.cmd_pipeline_barrier(
            self.cmd_buf,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[], &[], &[barrier],
        );

        if self.device.end_command_buffer(self.cmd_buf).is_err() {
            return -1;
        }

        let cmd_bufs = [self.cmd_buf];
        let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_bufs);

        if self.device.queue_submit(self.queue, &[submit_info], self.submit_fence).is_err() {
            return -1;
        }
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
            // Wait for the last signal_and_export_semaphore submission to finish.
            // On Windows, destroying a semaphore or command pool while GPU work
            // referencing them is still pending causes an access violation.
            if self.submit_fence_pending {
                let _ = self.device.wait_for_fences(
                    &[self.submit_fence],
                    true,
                    5_000_000_000, // 5 seconds
                );
                self.submit_fence_pending = false;
            }

            // Also do a full device wait as a safety net — Skia may have
            // submitted additional GPU work we don't track with our fence.
            self.device.device_wait_idle().ok();

            // Now safe to destroy everything.
            self.render_semaphore = None;
            self.device.destroy_fence(self.submit_fence, None);
            self.device.destroy_command_pool(self.cmd_pool, None);
            self.device.free_memory(self.memory, None);
            self.device.destroy_image(self.image, None);
        }
    }
}

// ── ImportedVkImage: RAII wrapper for resources imported by the parent ──────

/// Owns a VkImage + VkDeviceMemory imported from a tab process.
/// Cleaned up when dropped.
pub struct ImportedVkImage {
    device: ash::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
}

impl ImportedVkImage {
    /// The raw `VkImage` handle, needed for pipeline barriers.
    #[inline]
    pub fn image(&self) -> vk::Image {
        self.image
    }
}

impl Drop for ImportedVkImage {
    fn drop(&mut self) {
        unsafe {
            self.device.free_memory(self.memory, None);
            self.device.destroy_image(self.image, None);
        }
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
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: &ash::Device,
    handle: u64,
    width: u32,
    height: u32,
    format: vk::Format,
    alloc_size: u64,
) -> Result<ImportedVkImage, String> {
    let handle_type = external_handle_type();

    let mut ext_image_ci = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(handle_type);

    let image_ci = vk::ImageCreateInfo::default()
        .push_next(&mut ext_image_ci)
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D { width, height, depth: 1 })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(
            vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::TRANSFER_DST
                | vk::ImageUsageFlags::SAMPLED,
        )
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);

    let image = device
        .create_image(&image_ci, None)
        .map_err(|e| format!("vkCreateImage (raw import) failed: {:?}", e))?;

    let mem_reqs = device.get_image_memory_requirements(image);

    let imported_memory =
        import_memory(instance, physical_device, device, image, handle, alloc_size, &mem_reqs)?;

    device.bind_image_memory(image, imported_memory, 0).map_err(|e| {
        device.free_memory(imported_memory, None);
        device.destroy_image(image, None);
        format!("vkBindImageMemory (raw import) failed: {:?}", e)
    })?;

    Ok(ImportedVkImage {
        device: device.clone(),
        image,
        memory: imported_memory,
    })
}

// ── Platform-specific memory import helpers ─────────────────────────────────

#[cfg(windows)]
unsafe fn import_memory(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: &ash::Device,
    image: vk::Image,
    handle: u64,
    alloc_size: u64,
    mem_reqs: &vk::MemoryRequirements,
) -> Result<vk::DeviceMemory, String> {
    use ash::khr::external_memory_win32;

    let ext = external_memory_win32::Device::new(instance, device);

    let mut handle_props = vk::MemoryWin32HandlePropertiesKHR::default();
    ext.get_memory_win32_handle_properties(
        vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32,
        handle as vk::HANDLE,
        &mut handle_props,
    )
    .map_err(|e| format!("vkGetMemoryWin32HandlePropertiesKHR failed: {:?}", e))?;

    let combined_bits = mem_reqs.memory_type_bits & handle_props.memory_type_bits;
    let mem_type_index = find_memory_type(
        instance,
        physical_device,
        combined_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )
    .ok_or_else(|| {
        unsafe { device.destroy_image(image, None) };
        "No compatible memory type for import (win32)".to_string()
    })?;

    let mut import_info = vk::ImportMemoryWin32HandleInfoKHR::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32)
        .handle(handle as vk::HANDLE);

    let mut dedicated_alloc_info = vk::MemoryDedicatedAllocateInfo::default()
        .image(image);

    let alloc_info = vk::MemoryAllocateInfo::default()
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
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: &ash::Device,
    image: vk::Image,
    handle: u64,
    alloc_size: u64,
    mem_reqs: &vk::MemoryRequirements,
) -> Result<vk::DeviceMemory, String> {
    let fd = handle as std::os::unix::io::RawFd;

    let ext_fd = ash::khr::external_memory_fd::Device::new(instance, device);

    let mut fd_props = vk::MemoryFdPropertiesKHR::default();
    ext_fd
        .get_memory_fd_properties(
            vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
            fd,
            &mut fd_props,
        )
        .map_err(|e| {
            unsafe { device.destroy_image(image, None) };
            format!("vkGetMemoryFdPropertiesKHR failed: {:?}", e)
        })?;

    let combined_bits = mem_reqs.memory_type_bits & fd_props.memory_type_bits;
    let candidate_bits = if combined_bits != 0 { combined_bits } else { fd_props.memory_type_bits };

    let mem_type_index = find_memory_type(
        instance,
        physical_device,
        candidate_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )
    .or_else(|| {
        find_memory_type(instance, physical_device, candidate_bits, vk::MemoryPropertyFlags::empty())
    })
    .ok_or_else(|| {
        unsafe { device.destroy_image(image, None) };
        format!(
            "No compatible memory type for import (dma-buf fd): \
             image bits=0x{:x}, fd bits=0x{:x}",
            mem_reqs.memory_type_bits, fd_props.memory_type_bits
        )
    })?;

    let mut import_info = vk::ImportMemoryFdInfoKHR::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
        .fd(fd);

    let mut dedicated_alloc_info = vk::MemoryDedicatedAllocateInfo::default()
        .image(image);

    let alloc_info = vk::MemoryAllocateInfo::default()
        .push_next(&mut import_info)
        .push_next(&mut dedicated_alloc_info)
        .allocation_size(alloc_size)
        .memory_type_index(mem_type_index);

    device
        .allocate_memory(&alloc_info, None)
        .map_err(|e| {
            unsafe { device.destroy_image(image, None) };
            format!("vkAllocateMemory (dma-buf fd import) failed: {:?}", e)
        })
}

// ── Helper: find_memory_type ────────────────────────────────────────────────

pub fn find_memory_type(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    type_filter: u32,
    properties: vk::MemoryPropertyFlags,
) -> Option<u32> {
    let mem_props = unsafe { instance.get_physical_device_memory_properties(physical_device) };
    for i in 0..mem_props.memory_type_count {
        let ty = mem_props.memory_types[i as usize];
        if (type_filter & (1 << i)) != 0 && ty.property_flags.contains(properties) {
            return Some(i);
        }
    }
    None
}

// ── TabVkSemaphore (child/tab side) ─────────────────────────────────────────

/// An exportable binary `VkSemaphore` owned by the tab process.
pub struct TabVkSemaphore {
    device: ash::Device,
    pub semaphore: vk::Semaphore,
    #[cfg(windows)]
    ext_sem_win32: ash::khr::external_semaphore_win32::Device,
    #[cfg(not(windows))]
    ext_sem_fd: ash::khr::external_semaphore_fd::Device,
}

impl TabVkSemaphore {
    /// Create a new exportable binary semaphore.
    pub unsafe fn new(instance: &ash::Instance, device: &ash::Device) -> Result<Self, String> {
        let handle_type = external_semaphore_handle_type();

        let mut export_info = vk::ExportSemaphoreCreateInfo::default()
            .handle_types(handle_type);

        let sem_ci = vk::SemaphoreCreateInfo::default()
            .push_next(&mut export_info);

        let semaphore = device
            .create_semaphore(&sem_ci, None)
            .map_err(|e| format!("vkCreateSemaphore (exportable) failed: {:?}", e))?;

        #[cfg(windows)]
        let ext_sem_win32 = ash::khr::external_semaphore_win32::Device::new(instance, device);
        #[cfg(not(windows))]
        let ext_sem_fd = ash::khr::external_semaphore_fd::Device::new(instance, device);

        Ok(Self {
            device: device.clone(),
            semaphore,
            #[cfg(windows)]
            ext_sem_win32,
            #[cfg(not(windows))]
            ext_sem_fd,
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
            let _ = parent_pid;
            let get_info = vk::SemaphoreGetFdInfoKHR::default()
                .semaphore(self.semaphore)
                .handle_type(handle_type);
            match self.ext_sem_fd.get_semaphore_fd(&get_info) {
                Ok(fd) => fd as i64,
                Err(e) => {
                    eprintln!("[TabVkSemaphore] vkGetSemaphoreFdKHR failed: {:?}", e);
                    -1
                }
            }
        }

        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::{
                CloseHandle, DuplicateHandle, HANDLE, DUPLICATE_SAME_ACCESS, INVALID_HANDLE_VALUE,
            };
            use windows_sys::Win32::System::Threading::{
                GetCurrentProcess, OpenProcess, PROCESS_DUP_HANDLE,
            };

            let get_info = vk::SemaphoreGetWin32HandleInfoKHR::default()
                .semaphore(self.semaphore)
                .handle_type(handle_type);
            let local_handle = match self.ext_sem_win32.get_semaphore_win32_handle(&get_info) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("[TabVkSemaphore] vkGetSemaphoreWin32HandleKHR failed: {:?}", e);
                    return 0;
                }
            };

            let parent_proc: HANDLE = OpenProcess(PROCESS_DUP_HANDLE, 0, parent_pid);
            if parent_proc == 0 as HANDLE || parent_proc == INVALID_HANDLE_VALUE {
                CloseHandle(local_handle as HANDLE);
                return 0;
            }

            let mut dup: HANDLE = 0 as HANDLE;
            let ok = DuplicateHandle(
                GetCurrentProcess(),
                local_handle as HANDLE,
                parent_proc,
                &mut dup,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            );
            CloseHandle(parent_proc);
            CloseHandle(local_handle as HANDLE);
            if ok == 0 { 0 } else { dup as i64 }
        }
    }
}

impl Drop for TabVkSemaphore {
    fn drop(&mut self) {
        unsafe { self.device.destroy_semaphore(self.semaphore, None); }
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
    physical_device: vk::PhysicalDevice,
) -> [u8; 16] {
    let mut id_props = vk::PhysicalDeviceIDProperties::default();
    let mut props2 = vk::PhysicalDeviceProperties2::default().push_next(&mut id_props);
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

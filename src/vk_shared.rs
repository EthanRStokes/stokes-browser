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
///  2. Calls `import_skia_image` to bind the memory into its own Vulkan device and
///     produce a `skia_safe::Image` that can be drawn directly onto the swapchain canvas.

use ash::vk::{self, Handle};
use skia_safe::gpu::{self, DirectContext};
use skia_safe::{ColorType, Image};
use skia_safe::gpu::vk::AllocFlag;

// ── Platform-specific external semaphore handle type ───────────────────────

#[cfg(windows)]
fn external_semaphore_handle_type() -> vk::ExternalSemaphoreHandleTypeFlags {
    vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_WIN32
}

#[cfg(not(windows))]
fn external_semaphore_handle_type() -> vk::ExternalSemaphoreHandleTypeFlags {
    vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD
}
// ── VkFormat ↔ Skia mappings ────────────────────────────────────────────────

pub fn vk_format_to_skia(fmt: vk::Format) -> Option<(skia_safe::gpu::vk::Format, ColorType)> {
    match fmt {
        vk::Format::B8G8R8A8_UNORM => Some((skia_safe::gpu::vk::Format::B8G8R8A8_UNORM, ColorType::BGRA8888)),
        vk::Format::R8G8B8A8_UNORM => Some((skia_safe::gpu::vk::Format::R8G8B8A8_UNORM, ColorType::RGBA8888)),
        vk::Format::B8G8R8A8_SRGB  => Some((skia_safe::gpu::vk::Format::B8G8R8A8_SRGB,  ColorType::BGRA8888)),
        vk::Format::R8G8B8A8_SRGB  => Some((skia_safe::gpu::vk::Format::R8G8B8A8_SRGB,  ColorType::RGBA8888)),
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
    pub queue_family_index: u32,

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

        let family_indices = [queue_family_index];

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
            .queue_family_indices(&family_indices)
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
        let alloc = skia_safe::gpu::vk::Alloc::from_device_memory(
            memory.as_raw() as _,
            0,                 // offset
            mem_reqs.size,     // size
            AllocFlag::empty(),                 // flags
        );

        let sk_image_info = skia_safe::gpu::vk::ImageInfo::new(
            image.as_raw() as _,
            alloc,
            skia_safe::gpu::vk::ImageTiling::OPTIMAL,
            skia_safe::gpu::vk::ImageLayout::UNDEFINED,
            skia_format,
            1,
            None,
            None,
            None,
            None,
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
        let cmd_pool = {
            let pool_ci = vk::CommandPoolCreateInfo::default()
                .queue_family_index(queue_family_index)
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
            device.create_command_pool(&pool_ci, None).map_err(|e| {
                device.free_memory(memory, None);
                device.destroy_image(image, None);
                format!("vkCreateCommandPool (tab) failed: {:?}", e)
            })?
        };

        let cmd_buf = {
            let alloc_info = vk::CommandBufferAllocateInfo::default()
                .command_pool(cmd_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);
            let bufs = device.allocate_command_buffers(&alloc_info).map_err(|e| {
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
    ///   The local handle is closed after duplication; the parent is responsible for
    ///   closing the duplicated handle after importing.
    ///
    /// **Linux**: returns a dup'd file-descriptor number (i64 cast to u64). The caller
    ///   is responsible for closing the fd after sending it.
    ///
    /// # Parameters
    /// * `parent_pid` – On Windows: the PID of the parent process that will import the handle.
    ///                  Ignored on Linux.
    pub unsafe fn export_handle(&self, parent_pid: u32) -> Result<u64, String> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::{CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, INVALID_HANDLE_VALUE};
            use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcess, PROCESS_DUP_HANDLE};

            // Get our own handle from Vulkan
            let get_info = vk::MemoryGetWin32HandleInfoKHR::default()
                .memory(self.memory)
                .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_WIN32);
            let local_handle = self
                .ext_mem_win32
                .get_memory_win32_handle(&get_info)
                .map_err(|e| format!("vkGetMemoryWin32HandleKHR failed: {:?}", e))?;

            // Duplicate the handle into the parent process
            let parent_proc = OpenProcess(PROCESS_DUP_HANDLE, 0, parent_pid) as vk::HANDLE;
            if parent_proc == 0 || parent_proc == INVALID_HANDLE_VALUE as vk::HANDLE {
                let err = std::io::Error::last_os_error();
                CloseHandle(local_handle as _);
                return Err(format!("OpenProcess({}) failed: {}", parent_pid, err));
            }

            let mut dup_handle: windows_sys::Win32::Foundation::HANDLE = 0 as _;
            let ok = DuplicateHandle(
                GetCurrentProcess(),
                local_handle as _,
                parent_proc as _,
                &mut dup_handle,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            );
            CloseHandle(parent_proc as _);
            CloseHandle(local_handle as _);

            if ok == 0 {
                let err = std::io::Error::last_os_error();
                return Err(format!("DuplicateHandle failed: {}", err));
            }

            Ok(dup_handle as u64)
        }

        #[cfg(not(windows))]
        {
            let _ = parent_pid; // unused
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
    /// Returns the exported semaphore handle to embed in the IPC message, or
    /// -1/0 if the semaphore is unavailable.
    ///
    /// Must be called *after* `gr_context.flush_and_submit()`.
    pub unsafe fn signal_and_export_semaphore(&self, parent_pid: u32) -> i64 {
        let sem = match &self.render_semaphore {
            Some(s) => s,
            None => return -1,
        };

        // Re-record the command buffer each frame (pool has RESET_COMMAND_BUFFER).
        let begin_info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        if self.device.begin_command_buffer(self.cmd_buf, &begin_info).is_err() {
            return -1;
        }

        // Transition: COLOR_ATTACHMENT_OPTIMAL → GENERAL
        // (GENERAL is the safe layout for cross-process export; the parent will
        //  then transition it to SHADER_READ_ONLY_OPTIMAL before sampling.)
        let barrier = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::empty())
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });

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

        // Submit: execute barrier, then signal the semaphore.
        let signal_semaphores = [sem.semaphore];
        let cmd_bufs = [self.cmd_buf];
        let submit_info = vk::SubmitInfo::default()
            .command_buffers(&cmd_bufs)
            .signal_semaphores(&signal_semaphores);

        if self.device.queue_submit(self.queue, &[submit_info], vk::Fence::null()).is_err() {
            return -1;
        }

        // Export the (now-signaled) semaphore handle.
        sem.export(parent_pid)
    }
}

impl Drop for TabVkImage {
    fn drop(&mut self) {
        // Drop Skia surface first (holds GPU references)
        self.skia_surface = None;
        unsafe {
            self.device.device_wait_idle().ok();
            self.render_semaphore = None;
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

/// Import a cross-process memory handle into the parent's Vulkan device and wrap
/// it as a Skia `Image` ready to composite onto the swapchain canvas.
///
/// # Parameters
/// * `handle` – Platform handle value from `TabVkImage::export_handle`.
///   * Windows: a Win32 HANDLE already duplicated into this process.
///   * Linux:   a file descriptor number.
/// * `parent_pid` – Ignored; present for API symmetry.
///
/// # Safety
/// The caller must not use `handle` after this call; ownership is transferred to
/// the Vulkan driver.
pub unsafe fn import_skia_image(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: &ash::Device,
    gr_context: &mut DirectContext,
    handle: u64,
    width: u32,
    height: u32,
    format: vk::Format,
    alloc_size: u64,
) -> Result<(Image, ImportedVkImage), String> {
    let (skia_format, color_type) = vk_format_to_skia(format)
        .ok_or_else(|| format!("Unsupported format {:?}", format))?;

    let handle_type = external_handle_type();

    // -- Create the parent-side VkImage with external memory chain -----------
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
        .map_err(|e| format!("vkCreateImage (parent import) failed: {:?}", e))?;

    let mem_reqs = device.get_image_memory_requirements(image);

    // -- Import handle into VkDeviceMemory -----------------------------------
    // Use the original allocation size from the tab process, not the parent's
    // mem_reqs.size, which may differ due to alignment/padding differences.
    let imported_memory = import_memory(instance, physical_device, device, image, handle, alloc_size, &mem_reqs)?;

    device.bind_image_memory(image, imported_memory, 0).map_err(|e| {
        device.free_memory(imported_memory, None);
        device.destroy_image(image, None);
        format!("vkBindImageMemory (import) failed: {:?}", e)
    })?;

    // Wrap in RAII guard so it's cleaned up if Skia fails
    let guard = ImportedVkImage {
        device: device.clone(),
        image,
        memory: imported_memory,
    };

    // -- Build a Skia texture from the parent's VkImage ----------------------
    let alloc = skia_safe::gpu::vk::Alloc::from_device_memory(
        imported_memory.as_raw() as _,
        0,                 // offset
        alloc_size,        // size
        AllocFlag::empty(),                 // flags
    );

    let sk_image_info = skia_safe::gpu::vk::ImageInfo::new(
        image.as_raw() as _,
        alloc,
        skia_safe::gpu::vk::ImageTiling::OPTIMAL,
        skia_safe::gpu::vk::ImageLayout::UNDEFINED,
        skia_format,
        1,
        None,
        None,
        None,
        None,
    );

    let backend_texture = skia_safe::gpu::backend_textures::make_vk(
        (width as i32, height as i32),
        &sk_image_info,
        "tab_frame",
    );

    // The ImportedVkImage guard is returned to the caller so it stays alive
    // as long as the Skia image is in use, and gets cleaned up when dropped.

    let skia_image = gpu::images::borrow_texture_from(
        gr_context,
        &backend_texture,
        gpu::SurfaceOrigin::TopLeft,
        color_type,
        skia_safe::AlphaType::Premul,
        None,
    )
    .ok_or_else(|| "Failed to create Skia image from imported Vulkan texture".to_string())?;

    Ok((skia_image, guard))
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

    // Query memory type bits for the imported handle
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

    // DMA_BUF handles support vkGetMemoryFdPropertiesKHR (unlike OPAQUE_FD),
    // which tells us which memory types are compatible with this imported fd.
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

    // Prefer the intersection of image-required and fd-compatible types, but
    // fall back to fd-only bits if the intersection is empty (common on AMD/Intel
    // where DMA-BUF memory lives in a GTT/system heap the image doesn't list).
    let combined_bits = mem_reqs.memory_type_bits & fd_props.memory_type_bits;
    let candidate_bits = if combined_bits != 0 { combined_bits } else { fd_props.memory_type_bits };

    let mem_type_index = find_memory_type(
        instance,
        physical_device,
        candidate_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )
    .or_else(|| {
        // Fallback: try without DEVICE_LOCAL requirement
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
///
/// The tab submits GPU work that *signals* this semaphore, then exports it as
/// a platform handle sent to the parent alongside the memory handle.  The
/// parent imports and *waits* on it so the GPU pipeline stalls until the tab's
/// render is complete before reading the shared image.
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
    /// * Linux   – returns a `sync_fd` (i32 cast to i64). SYNC_FD semaphores
    ///             are one-shot: the fd is consumed on import and the semaphore
    ///             automatically resets to unsignaled afterward.
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
            use windows_sys::Win32::Foundation::{CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, INVALID_HANDLE_VALUE};
            use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcess, PROCESS_DUP_HANDLE};

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

            let parent_proc = OpenProcess(PROCESS_DUP_HANDLE, 0, parent_pid) as vk::HANDLE;
            if parent_proc == 0 || parent_proc == INVALID_HANDLE_VALUE as vk::HANDLE {
                CloseHandle(local_handle as _);
                return 0;
            }
            let mut dup: windows_sys::Win32::Foundation::HANDLE = 0;
            let ok = DuplicateHandle(
                GetCurrentProcess(), local_handle as _, parent_proc as _, &mut dup,
                0, 0, DUPLICATE_SAME_ACCESS,
            );
            CloseHandle(parent_proc as _);
            CloseHandle(local_handle as _);
            if ok == 0 { 0 } else { dup as i64 }
        }
    }
}

impl Drop for TabVkSemaphore {
    fn drop(&mut self) {
        unsafe { self.device.destroy_semaphore(self.semaphore, None); }
    }
}

// ── Parent-side semaphore import + GPU wait ──────────────────────────────────

/// Import a cross-process semaphore handle and submit a GPU wait on it so the
/// parent's pipeline stalls until the tab's render is complete.
///
/// A command buffer is submitted to `queue` that:
///   1. Waits on the imported semaphore at `FRAGMENT_SHADER` stage.
///   2. Executes a pipeline barrier that transitions `image` from
///      `GENERAL` → `SHADER_READ_ONLY_OPTIMAL`.
///
/// On success returns the temporary `VkSemaphore` (which will be auto-reset
/// after the wait on Linux SYNC_FD, or must be destroyed after the submit
/// completes on Windows).  The caller should `device_wait_idle` or use a fence
/// before dropping the returned semaphore if on Windows.
///
/// Falls back silently (no barrier submitted) when `sem_handle` is -1/0.
pub unsafe fn import_semaphore_and_submit_wait(
    instance: &ash::Instance,
    device: &ash::Device,
    queue: vk::Queue,
    queue_family_index: u32,
    image: vk::Image,
    sem_handle: i64,
) -> Result<Option<vk::Semaphore>, String> {
    // -1 (Linux) / 0 (Windows) → no semaphore was sent, fall through.
    #[cfg(not(windows))]
    if sem_handle == -1 { return Ok(None); }
    #[cfg(windows)]
    if sem_handle == 0 { return Ok(None); }

    // -- Import the semaphore ------------------------------------------------
    let semaphore = {
        let sem_ci = vk::SemaphoreCreateInfo::default();
        device.create_semaphore(&sem_ci, None)
            .map_err(|e| format!("vkCreateSemaphore (import) failed: {:?}", e))?
    };

    #[cfg(not(windows))]
    {
        let ext_fd = ash::khr::external_semaphore_fd::Device::new(instance, device);
        let import_info = vk::ImportSemaphoreFdInfoKHR::default()
            .semaphore(semaphore)
            .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD)
            // TEMPORARY: semaphore auto-resets to unsignaled after the wait.
            .flags(vk::SemaphoreImportFlags::TEMPORARY)
            .fd(sem_handle as i32);
        ext_fd.import_semaphore_fd(&import_info)
            .map_err(|e| { device.destroy_semaphore(semaphore, None); format!("vkImportSemaphoreFdKHR failed: {:?}", e) })?;
    }

    #[cfg(windows)]
    {
        let ext_win32 = ash::khr::external_semaphore_win32::Device::new(instance, device);
        let import_info = vk::ImportSemaphoreWin32HandleInfoKHR::default()
            .semaphore(semaphore)
            .handle_type(vk::ExternalSemaphoreHandleTypeFlags::OPAQUE_WIN32)
            .flags(vk::SemaphoreImportFlags::TEMPORARY)
            .handle(sem_handle as vk::HANDLE);
        ext_win32.import_semaphore_win32_handle(&import_info)
            .map_err(|e| { device.destroy_semaphore(semaphore, None); format!("vkImportSemaphoreWin32HandleKHR failed: {:?}", e) })?;
    }

    // -- Allocate a one-shot command buffer ----------------------------------
    let cmd_pool = {
        let pool_ci = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family_index)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        device.create_command_pool(&pool_ci, None)
            .map_err(|e| { device.destroy_semaphore(semaphore, None); format!("vkCreateCommandPool (sem wait) failed: {:?}", e) })?
    };

    let cmd_buf = {
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(cmd_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let bufs = device.allocate_command_buffers(&alloc_info)
            .map_err(|e| { device.destroy_command_pool(cmd_pool, None); device.destroy_semaphore(semaphore, None); format!("vkAllocateCommandBuffers failed: {:?}", e) })?;
        bufs[0]
    };

    // -- Record: image layout transition GENERAL → SHADER_READ_ONLY_OPTIMAL -
    let begin_info = vk::CommandBufferBeginInfo::default()
        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    device.begin_command_buffer(cmd_buf, &begin_info)
        .map_err(|e| format!("vkBeginCommandBuffer failed: {:?}", e))?;

    let barrier = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE | vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ)
        .old_layout(vk::ImageLayout::GENERAL)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    device.cmd_pipeline_barrier(
        cmd_buf,
        vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT | vk::PipelineStageFlags::TRANSFER,
        vk::PipelineStageFlags::FRAGMENT_SHADER,
        vk::DependencyFlags::empty(),
        &[],
        &[],
        &[barrier],
    );

    device.end_command_buffer(cmd_buf)
        .map_err(|e| format!("vkEndCommandBuffer failed: {:?}", e))?;

    // -- Submit: wait on imported semaphore, execute barrier -----------------
    let wait_semaphores = [semaphore];
    let wait_stages = [vk::PipelineStageFlags::FRAGMENT_SHADER];
    let cmd_bufs = [cmd_buf];

    let submit_info = vk::SubmitInfo::default()
        .wait_semaphores(&wait_semaphores)
        .wait_dst_stage_mask(&wait_stages)
        .command_buffers(&cmd_bufs);

    // Use a fence so we can clean up the pool when done.
    let fence = device.create_fence(&vk::FenceCreateInfo::default(), None)
        .map_err(|e| format!("vkCreateFence (sem wait) failed: {:?}", e))?;

    device.queue_submit(queue, &[submit_info], fence)
        .map_err(|e| { device.destroy_fence(fence, None); format!("vkQueueSubmit (sem wait) failed: {:?}", e) })?;

    // Wait for the barrier submit to complete so we can free the command pool.
    // This is a one-time cost per frame; the GPU does the heavy waiting async.
    device.wait_for_fences(&[fence], true, 5_000_000_000 /* 5 s */)
        .map_err(|e| format!("wait_for_fences (sem wait) failed: {:?}", e))?;

    device.destroy_fence(fence, None);
    device.destroy_command_pool(cmd_pool, None);

    Ok(Some(semaphore))
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


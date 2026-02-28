/// Shared-Vulkan-image helpers for zero-copy cross-process rendering.
///
/// # Platform support
/// * **Windows** – uses `VK_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_WIN32_BIT` + `DuplicateHandle`.
/// * **Linux**   – uses `VK_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_FD_BIT`.
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
    vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD
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
    skia_surface: Option<skia_safe::Surface>,

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

        let alloc_info = vk::MemoryAllocateInfo::default()
            .push_next(&mut export_alloc_info)
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
        let alloc = skia_safe::gpu::vk::Alloc::default();
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
            skia_surface: Some(skia_surface),
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
                .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);

            self.ext_mem_fd
                .get_memory_fd(&get_fd_info)
                .map(|fd| fd as u64)
                .map_err(|e| format!("vkGetMemoryFdKHR failed: {:?}", e))
        }
    }
}

impl Drop for TabVkImage {
    fn drop(&mut self) {
        // Drop Skia surface first (holds GPU references)
        self.skia_surface = None;
        unsafe {
            self.device.free_memory(self.memory, None);
            self.device.destroy_image(self.image, None);
        }
    }
}

// ── ImportedVkImage: RAII wrapper for resources imported by the parent ──────

/// Owns a VkImage + VkDeviceMemory imported from a tab process.
/// Cleaned up when dropped.
struct ImportedVkImage {
    device: ash::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
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
) -> Result<Image, String> {
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
    let imported_memory = import_memory(instance, physical_device, device, image, handle, &mem_reqs)?;

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
    let alloc = skia_safe::gpu::vk::Alloc::default();
    let sk_image_info = skia_safe::gpu::vk::ImageInfo::new(
        image.as_raw() as _,
        alloc,
        skia_safe::gpu::vk::ImageTiling::OPTIMAL,
        skia_safe::gpu::vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
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

    // Use a release callback so that the VkImage/VkDeviceMemory are freed
    // exactly when Skia is done with the texture.
    // For now, leak the guard so the imported resources stay alive.
    // TODO: use a release callback once the Skia API supports it.
    std::mem::forget(guard);

    let skia_image = gpu::images::borrow_texture_from(
        gr_context,
        &backend_texture,
        gpu::SurfaceOrigin::TopLeft,
        color_type,
        skia_safe::AlphaType::Premul,
        None,
    )
    .ok_or_else(|| "Failed to create Skia image from imported Vulkan texture".to_string())?;

    Ok(skia_image)
}

/// Release callback invoked by Skia when it is done with the imported texture.
/*unsafe extern "C" fn release_imported_vk_image(
    _texture: skia_safe::gpu::BackendTexture,
    _release: skia_safe::gpu::SurfaceReleaseProc,
    context: *mut std::ffi::c_void,
) {
    if !context.is_null() {
        // Re-take ownership and drop, which frees the VkImage + VkDeviceMemory.
        let _ = Box::from_raw(context as *mut ImportedVkImage);
    }
}*/

// ── Platform-specific memory import helpers ─────────────────────────────────

#[cfg(windows)]
unsafe fn import_memory(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: &ash::Device,
    image: vk::Image,
    handle: u64,
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

    let alloc_info = vk::MemoryAllocateInfo::default()
        .push_next(&mut import_info)
        .allocation_size(mem_reqs.size)
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
    mem_reqs: &vk::MemoryRequirements,
) -> Result<vk::DeviceMemory, String> {
    let fd = handle as std::os::unix::io::RawFd;
    let ext_mem_fd = ash::khr::external_memory_fd::Device::new(instance, device);

    let mut fd_props = vk::MemoryFdPropertiesKHR::default();
    ext_mem_fd
        .get_memory_fd_properties(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD, fd, &mut fd_props)
        .map_err(|e| format!("vkGetMemoryFdPropertiesKHR failed: {:?}", e))?;

    let combined_bits = mem_reqs.memory_type_bits & fd_props.memory_type_bits;
    let mem_type_index = find_memory_type(
        instance,
        physical_device,
        combined_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )
    .ok_or_else(|| {
        unsafe { device.destroy_image(image, None) };
        "No compatible memory type for import (fd)".to_string()
    })?;

    let mut import_info = vk::ImportMemoryFdInfoKHR::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD)
        .fd(fd);

    let alloc_info = vk::MemoryAllocateInfo::default()
        .push_next(&mut import_info)
        .allocation_size(mem_reqs.size)
        .memory_type_index(mem_type_index);

    device
        .allocate_memory(&alloc_info, None)
        .map_err(|e| {
            unsafe { device.destroy_image(image, None) };
            format!("vkAllocateMemory (fd import) failed: {:?}", e)
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

// ── VulkanDeviceInfo: serialisable info the parent passes to child via env ──

/// The minimal Vulkan handles the parent passes to each tab process so they can
/// connect to the same physical device.
///
/// All u64 values are raw Vulkan opaque handles transmitted as integers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VulkanDeviceInfo {
    /// Raw `VkPhysicalDevice` handle — used to select the same GPU in the child.
    pub physical_device_handle: u64,
    /// Queue family index used by the parent.
    pub queue_family_index: u32,
    /// Swapchain image format (as raw `VkFormat` integer).
    pub image_format: i32,
    /// PID of the parent process (needed on Windows for `DuplicateHandle`).
    pub parent_pid: u32,
}

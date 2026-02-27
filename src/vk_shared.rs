/// Shared-Vulkan-image helpers for zero-copy cross-process rendering.
///
/// # How it works
///
/// The tab (child) process:
///  1. Calls `TabVkImage::new` to allocate a `VkImage` backed by exportable memory
///     (`VK_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_FD_BIT_KHR`).
///  2. Creates a Skia GPU surface that wraps the image for rendering.
///  3. After each rendered frame, calls `export_fd` to get a dup'd file descriptor,
///     and sends it together with the raw `VkImage` handle over IPC.
///
/// The parent process:
///  1. Receives the fd + handle via `FrameRendered`.
///  2. Calls `import_skia_image` to bind the memory into its own Vulkan device and
///     produce a `skia_safe::Image` that can be drawn directly onto the swapchain canvas.

use std::os::unix::io::RawFd;
use ash::vk::{self, Handle};
use skia_safe::gpu::{self, DirectContext};
use skia_safe::{ColorType, Image};

// ── VkFormat ↔ Skia mappings (same as window.rs) ──────────────────────────

pub fn vk_format_to_skia(fmt: vk::Format) -> Option<(skia_safe::gpu::vk::Format, ColorType)> {
    match fmt {
        vk::Format::B8G8R8A8_UNORM => Some((skia_safe::gpu::vk::Format::B8G8R8A8_UNORM, ColorType::BGRA8888)),
        vk::Format::R8G8B8A8_UNORM => Some((skia_safe::gpu::vk::Format::R8G8B8A8_UNORM, ColorType::RGBA8888)),
        vk::Format::B8G8R8A8_SRGB  => Some((skia_safe::gpu::vk::Format::B8G8R8A8_SRGB,  ColorType::BGRA8888)),
        vk::Format::R8G8B8A8_SRGB  => Some((skia_safe::gpu::vk::Format::R8G8B8A8_SRGB,  ColorType::RGBA8888)),
        _ => None,
    }
}

// ── TabVkImage (child/tab side) ────────────────────────────────────────────

/// A Vulkan image + device-memory pair owned by the tab process.
/// Memory is allocated with `VK_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_FD_BIT_KHR`
/// so a dup'd fd can be sent to the parent.
pub struct TabVkImage {
    pub instance: ash::Instance,
    pub device: ash::Device,
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
    pub width: u32,
    pub height: u32,
    pub format: vk::Format,
    pub layout: vk::ImageLayout,
    /// The ash extension loader for `VK_KHR_external_memory_fd`.
    ext_mem_fd: ash::khr::external_memory_fd::Device,
    /// Keep a Skia surface so we can render into the image GPU-side.
    skia_surface: Option<skia_safe::Surface>,
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

        // -- Create image with external memory info chained in pNext -----------
        let mut ext_image_ci = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);

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
            .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .queue_family_indices(&family_indices)
            .initial_layout(vk::ImageLayout::UNDEFINED);

        let image = device.create_image(&image_ci, None)
            .map_err(|e| format!("vkCreateImage failed: {:?}", e))?;

        // -- Allocate memory with export info -----------------------------------
        let mem_reqs = device.get_image_memory_requirements(image);

        let mem_type_index = find_memory_type(
            instance, physical_device,
            mem_reqs.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        ).ok_or_else(|| "No suitable memory type".to_string())?;

        let mut export_alloc_info = vk::ExportMemoryAllocateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);

        let alloc_info = vk::MemoryAllocateInfo::default()
            .push_next(&mut export_alloc_info)
            .allocation_size(mem_reqs.size)
            .memory_type_index(mem_type_index);

        let memory = device.allocate_memory(&alloc_info, None)
            .map_err(|e| format!("vkAllocateMemory failed: {:?}", e))?;

        device.bind_image_memory(image, memory, 0)
            .map_err(|e| format!("vkBindImageMemory failed: {:?}", e))?;

        // -- Transition to COLOR_ATTACHMENT_OPTIMAL ---------------------------
        // (done lazily by Skia; we leave the image in UNDEFINED for now)

        // -- Build Skia surface ------------------------------------------------
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
        ).ok_or_else(|| "Failed to wrap Vulkan image as Skia surface".to_string())?;

        let ext_mem_fd = ash::khr::external_memory_fd::Device::new(instance, device);

        Ok(Self {
            instance: instance.clone(),
            device: device.clone(),
            image,
            memory,
            width,
            height,
            format,
            layout: vk::ImageLayout::UNDEFINED,
            ext_mem_fd,
            skia_surface: Some(skia_surface),
        })
    }

    /// Get a mutable reference to the Skia surface for rendering.
    pub fn surface_mut(&mut self) -> &mut skia_safe::Surface {
        self.skia_surface.as_mut().expect("Skia surface not initialised")
    }

    /// Export a dup'd opaque fd representing the backing device memory.
    /// The caller takes ownership of the returned fd and is responsible for closing it.
    pub unsafe fn export_fd(&self) -> Result<RawFd, String> {
        let get_fd_info = vk::MemoryGetFdInfoKHR::default()
            .memory(self.memory)
            .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);

        self.ext_mem_fd.get_memory_fd(&get_fd_info)
            .map_err(|e| format!("vkGetMemoryFdKHR failed: {:?}", e))
    }
}

impl Drop for TabVkImage {
    fn drop(&mut self) {
        // Drop the Skia surface first (it holds references to device objects)
        self.skia_surface = None;
        unsafe {
            self.device.free_memory(self.memory, None);
            self.device.destroy_image(self.image, None);
        }
    }
}

// ── Parent-side import ─────────────────────────────────────────────────────

/// Import a memory fd into the parent's Vulkan device and wrap it as a
/// Skia `Image` ready to composite onto the swapchain canvas.
///
/// The parent creates its own `VkImage` with identical parameters, then
/// binds the imported memory to it — this is the correct way to share a
/// VkImage across processes using opaque fd handle semantics.
///
/// # Safety
/// `fd` is consumed (closed) by the Vulkan driver after import.
pub unsafe fn import_skia_image(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: &ash::Device,
    gr_context: &mut DirectContext,
    fd: RawFd,
    _vk_image_raw: u64,  // informational only; the parent creates its own image
    width: u32,
    height: u32,
    format: vk::Format,
) -> Result<Image, String> {
    let (skia_format, color_type) = vk_format_to_skia(format)
        .ok_or_else(|| format!("Unsupported format {:?}", format))?;

    let ext_mem_fd = ash::khr::external_memory_fd::Device::new(instance, device);

    // Query memory type bits compatible with the imported fd
    let mut fd_props = vk::MemoryFdPropertiesKHR::default();
    ext_mem_fd
        .get_memory_fd_properties(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD, fd, &mut fd_props)
        .map_err(|e| format!("vkGetMemoryFdPropertiesKHR failed: {:?}", e))?;

    // Create the parent-side VkImage with external memory chain
    let mut ext_image_ci = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD);
    let image_ci = vk::ImageCreateInfo::default()
        .push_next(&mut ext_image_ci)
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D { width, height, depth: 1 })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);

    let image = device.create_image(&image_ci, None)
        .map_err(|e| format!("vkCreateImage (parent import) failed: {:?}", e))?;

    let mem_reqs = device.get_image_memory_requirements(image);

    // Pick a memory type that satisfies both the image requirements and the fd properties
    let combined_bits = mem_reqs.memory_type_bits & fd_props.memory_type_bits;
    let mem_type_index = find_memory_type(
        instance, physical_device,
        combined_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    ).ok_or_else(|| {
        device.destroy_image(image, None);
        "No compatible memory type for import".to_string()
    })?;

    // Import the fd into a VkDeviceMemory
    let mut import_info = vk::ImportMemoryFdInfoKHR::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::OPAQUE_FD)
        .fd(fd);

    let alloc_info = vk::MemoryAllocateInfo::default()
        .push_next(&mut import_info)
        .allocation_size(mem_reqs.size)
        .memory_type_index(mem_type_index);

    let imported_memory = device.allocate_memory(&alloc_info, None)
        .map_err(|e| {
            device.destroy_image(image, None);
            format!("vkAllocateMemory (import) failed: {:?}", e)
        })?;

    device.bind_image_memory(image, imported_memory, 0)
        .map_err(|e| {
            device.free_memory(imported_memory, None);
            device.destroy_image(image, None);
            format!("vkBindImageMemory (import) failed: {:?}", e)
        })?;

    // Build a Skia texture from the parent's VkImage
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

    let skia_image = Image::from_texture(
        gr_context,
        &backend_texture,
        gpu::SurfaceOrigin::TopLeft,
        color_type,
        skia_safe::AlphaType::Premul,
        None,
    ).ok_or_else(|| {
        // Clean up on failure
        device.free_memory(imported_memory, None);
        device.destroy_image(image, None);
        "Failed to create Skia image from imported Vulkan texture".to_string()
    })?;

    // TODO: The VkImage and VkDeviceMemory need to be freed when `skia_image` is
    // no longer in use.  For now we accept the small leak; a real implementation
    // would wrap them in an Arc with a drop hook.
    // `imported_memory` and `image` are intentionally leaked here.

    Ok(skia_image)
}

// ── Helper: find_memory_type ───────────────────────────────────────────────

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

// ── VulkanDeviceInfo: serialisable info the parent passes to child via CLI ──

/// The minimal Vulkan handles the parent passes to each tab process so they can
/// connect to the same physical device.
///
/// All u64 values are raw Vulkan opaque handles transmitted as integers.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VulkanDeviceInfo {
    /// Raw `VkInstance` handle
    pub instance_handle: u64,
    /// Raw `VkPhysicalDevice` handle
    pub physical_device_handle: u64,
    /// Queue family index used by the parent
    pub queue_family_index: u32,
    /// Swapchain image format (as raw `VkFormat` integer)
    pub image_format: i32,
}





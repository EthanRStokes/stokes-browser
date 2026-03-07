use std::sync::Arc;
use vulkano::device::{physical::PhysicalDeviceType, Device, DeviceCreateInfo, DeviceExtensions, DeviceFeatures, Queue, QueueCreateInfo, QueueFlags};
use vulkano::instance::{Instance, InstanceCreateInfo};
use vulkano::swapchain::Surface;
use vulkano::VulkanLibrary;
use vulkano::device::physical::PhysicalDevice;
use winit_core::event_loop::ActiveEventLoop;
use winit_core::window::Window;

pub(crate) struct VulkanoOwnedContext {
    pub(crate) _library: Arc<VulkanLibrary>,
    pub(crate) instance_owner: Arc<Instance>,
    pub(crate) device_owner: Arc<Device>,
    pub(crate) queue_owner: Arc<Queue>,
    pub(crate) surface_owner: Option<Arc<Surface>>,
    pub(crate) physical_device: Arc<PhysicalDevice>,
    pub(crate) queue_family_index: u32,
    pub(crate) negotiated_api_version: u32,
}

pub(crate) fn create_parent_context(window: Arc<Box<dyn Window>>, el: &Box<&dyn ActiveEventLoop>) -> Result<VulkanoOwnedContext, String> {
    let library = VulkanLibrary::new().map_err(|e| format!("VulkanLibrary::new: {e:?}"))?;

    let instance_extensions = Surface::required_extensions(el)
        .map_err(|e| format!("Surface::required_extensions: {e}"))?;

    let instance = Instance::new(
        library.clone(),
        InstanceCreateInfo {
            enabled_extensions: instance_extensions,
            ..Default::default()
        },
    )
    .map_err(|e| format!("Instance::new (parent): {e:?}"))?;

    let surface = unsafe { Surface::from_window_ref(instance.clone(), &*window) }
        .map_err(|e| format!("Surface::from_window_ref: {e:?}"))?;

    let required_device_extensions = parent_device_extensions();

    let (physical_device, queue_family_index) = instance
        .enumerate_physical_devices()
        .map_err(|e| format!("enumerate_physical_devices (parent): {e:?}"))?
        .filter(|pd| pd.supported_extensions().contains(&required_device_extensions))
        .filter_map(|pd| {
            pd.queue_family_properties()
                .iter()
                .enumerate()
                .position(|(i, q)| {
                    q.queue_flags.intersects(QueueFlags::GRAPHICS)
                        && pd.surface_support(i as u32, &surface).unwrap_or(false)
                })
                .map(|i| (pd, i as u32))
        })
        .min_by_key(|(pd, _)| match pd.properties().device_type {
            PhysicalDeviceType::DiscreteGpu => 0,
            PhysicalDeviceType::IntegratedGpu => 1,
            PhysicalDeviceType::VirtualGpu => 2,
            PhysicalDeviceType::Cpu => 3,
            _ => 4,
        })
        .ok_or_else(|| "No suitable Vulkan physical device found (parent)".to_string())?;

    #[cfg(target_os = "windows")]
    let windows = true;
    #[cfg(not(target_os = "windows"))]
    let windows = false;

    let (device, mut queues) = Device::new(
        physical_device.clone(),
        DeviceCreateInfo {
            enabled_extensions: required_device_extensions,
            enabled_features: DeviceFeatures {
                synchronization2: !windows,
                ..DeviceFeatures::empty()
            },
            queue_create_infos: vec![QueueCreateInfo {
                queue_family_index,
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .map_err(|e| format!("Device::new (parent): {e:?}"))?;

    let queue = queues
        .next()
        .ok_or_else(|| "Device::new returned no queue (parent)".to_string())?;

    let negotiated_api_version = crate::vk_shared::negotiated_api_version(&instance, &physical_device);

    Ok(VulkanoOwnedContext {
        _library: library,
        instance_owner: instance,
        device_owner: device,
        queue_owner: queue.clone(),
        surface_owner: Some(surface),
        physical_device,
        queue_family_index,
        negotiated_api_version,
    })
}

pub(crate) fn create_tab_context(
    parent_info: Option<&crate::vk_shared::VulkanDeviceInfo>,
) -> Result<VulkanoOwnedContext, String> {
    let library = VulkanLibrary::new().map_err(|e| format!("VulkanLibrary::new: {e:?}"))?;

    let instance = Instance::new(library.clone(), InstanceCreateInfo::default())
        .map_err(|e| format!("Instance::new (tab): {e:?}"))?;

    let required_device_extensions = tab_device_extensions();

    let physical_devices: Vec<_> = instance
        .enumerate_physical_devices()
        .map_err(|e| format!("enumerate_physical_devices (tab): {e:?}"))?
        .filter(|pd| pd.supported_extensions().contains(&required_device_extensions))
        .collect();

    if physical_devices.is_empty() {
        return Err("No Vulkan physical devices with required tab extensions".to_string());
    }

    let selected = if let Some(info) = parent_info {
        physical_devices
            .iter()
            .find(|pd| crate::vk_shared::physical_device_uuid(pd) == info.device_uuid)
            .cloned()
            .unwrap_or_else(|| physical_devices[0].clone())
    } else {
        physical_devices[0].clone()
    };

    let queue_families = selected.queue_family_properties();
    let fallback_qfi = queue_families
        .iter()
        .enumerate()
        .find(|(_, q)| q.queue_flags.intersects(QueueFlags::GRAPHICS))
        .map(|(i, _)| i as u32)
        .ok_or_else(|| "No graphics queue family found (tab)".to_string())?;

    let queue_family_index = parent_info
        .and_then(|info| {
            queue_families
                .get(info.queue_family_index as usize)
                .filter(|q| q.queue_flags.intersects(QueueFlags::GRAPHICS))
                .map(|_| info.queue_family_index)
        })
        .unwrap_or(fallback_qfi);

    let (device, mut queues) = Device::new(
        selected.clone(),
        DeviceCreateInfo {
            enabled_extensions: required_device_extensions,
            queue_create_infos: vec![QueueCreateInfo {
                queue_family_index,
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .map_err(|e| format!("Device::new (tab): {e:?}"))?;

    let queue = queues
        .next()
        .ok_or_else(|| "Device::new returned no queue (tab)".to_string())?;

    let negotiated_api_version = crate::vk_shared::negotiated_api_version(&instance, &selected);

    Ok(VulkanoOwnedContext {
        _library: library,
        instance_owner: instance,
        device_owner: device,
        queue_owner: queue.clone(),
        surface_owner: None,
        physical_device: selected,
        queue_family_index,
        negotiated_api_version,
    })
}

fn parent_device_extensions() -> DeviceExtensions {
    let mut exts = DeviceExtensions {
        khr_swapchain: true,
        khr_external_memory: true,
        khr_external_semaphore: true,
        ..DeviceExtensions::empty()
    };

    #[cfg(windows)]
    {
        exts.khr_external_memory_win32 = true;
        exts.khr_external_semaphore_win32 = true;
    }

    #[cfg(not(windows))]
    {
        exts.khr_external_memory_fd = true;
        exts.ext_external_memory_dma_buf = true;
        exts.khr_external_semaphore_fd = true;
    }

    exts
}

fn tab_device_extensions() -> DeviceExtensions {
    let mut exts = DeviceExtensions {
        khr_external_memory: true,
        khr_external_semaphore: true,
        ..DeviceExtensions::empty()
    };

    #[cfg(windows)]
    {
        exts.khr_external_memory_win32 = true;
        exts.khr_external_semaphore_win32 = true;
    }

    #[cfg(not(windows))]
    {
        exts.khr_external_memory_fd = true;
        exts.ext_external_memory_dma_buf = true;
        exts.khr_external_semaphore_fd = true;
    }

    exts
}

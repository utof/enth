//! Snapshot harness: CPU rendering only (Mesa lavapipe through wgpu's Vulkan backend). Golden
//! images come from one renderer; a GPU would differ, and on this machine must not be touched.
#![allow(dead_code, reason = "each test binary uses a different subset")]

use std::sync::Arc;

use egui_kittest::{Harness, wgpu::WgpuTestRenderer};
use egui_wgpu::wgpu;
use enth_ui::Theme;

/// A wgpu renderer on a CPU adapter, or a panic that says how to get one.
pub fn cpu_renderer() -> WgpuTestRenderer {
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::VULKAN;
    setup.native_adapter_selector = Some(Arc::new(|adapters, _surface| {
        adapters
            .iter()
            .find(|a| a.get_info().device_type == wgpu::DeviceType::Cpu)
            .cloned()
            .ok_or_else(|| {
                let found: Vec<String> = adapters.iter().map(|a| a.get_info().name).collect();
                format!(
                    "no CPU Vulkan adapter (found {found:?}); install Mesa lavapipe \
                     (apt install mesa-vulkan-drivers) and check VK_ICD_FILENAMES"
                )
            })
    }));
    WgpuTestRenderer::from_setup(egui_wgpu::WgpuSetup::CreateNew(setup))
}

/// A harness at scale 1.0 with the theme applied before the first pass.
pub fn harness<'a, S>(
    theme: &Theme,
    size: egui::Vec2,
    app: impl FnMut(&mut egui::Ui, &mut S) + 'a,
    state: S,
) -> Harness<'a, S> {
    let theme = theme.clone();
    Harness::builder()
        .with_size(size)
        .with_pixels_per_point(1.0)
        .with_context_setup(move |ctx| theme.apply(ctx))
        .renderer(cpu_renderer())
        .build_ui_state(app, state)
}

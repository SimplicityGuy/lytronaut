//! Optional presentation of already verified, explicitly oriented derivatives.
//! Imported recipes are never interpreted here. The CPU JPEG remains authoritative.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub width: u32,
    pub height: u32,
    pub cpu_bytes: u64,
    pub gpu_bytes: u64,
}
pub fn allocation(
    width: u32,
    height: u32,
    device_limit: u32,
    quality: u32,
) -> Result<Allocation, &'static str> {
    if width == 0 || height == 0 || width > 2048 || height > 2048 || quality == 0 {
        return Err("GPU source exceeds decoded cap");
    }
    let cap = quality.min(1024).min(device_limit);
    if cap == 0 {
        return Err("GPU texture limit unavailable");
    }
    let longest = width.max(height);
    let w = ((u64::from(width) * u64::from(cap.min(longest))) / u64::from(longest)).max(1) as u32;
    let h = ((u64::from(height) * u64::from(cap.min(longest))) / u64::from(longest)).max(1) as u32;
    let rgba = u64::from(w) * u64::from(h) * 4;
    let upload_rgba = u64::from(width.min(1024)) * u64::from(height.min(1024)) * 4;
    // 32 MiB persistent DOM + eight bounded 2 MiB encoded copies (native
    // fallback and identity GPU source) + 3 MiB metadata/runtime allowance,
    // two source decodes, canvas/ImageData/WASM staging. At the actual DTO
    // limit of 512² the nonpersistent total fits the shared 24 MiB lease.
    // A future larger source must reserve its computed transient total first.
    // Queue upload staging is also counted on GPU.
    let cpu_bytes =
        51 * 1024 * 1024 + 2 * u64::from(width) * u64::from(height) * 4 + 3 * upload_rgba;
    let gpu_bytes = 2 * upload_rgba + 3 * rgba + 16; // input, upload staging, up to three surface images
    if cpu_bytes > 96 * 1024 * 1024 || gpu_bytes > 64 * 1024 * 1024 {
        return Err("GPU presentation exceeds allocation budget");
    }
    Ok(Allocation {
        width: w,
        height: h,
        cpu_bytes,
        gpu_bytes,
    })
}
#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use wasm_bindgen::JsCast;
    thread_local! {
        // Shared by every mounted owner; a timed-out native acquisition still
        // owns this slot while its promise drains and late device is destroyed.
        static ACQUIRING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    #[cfg(feature = "test-harness")]
    pub(crate) fn acquisition_pending() -> bool {
        ACQUIRING.with(std::cell::Cell::get)
    }
    struct Acquisition;
    impl Acquisition {
        fn take() -> Result<Self, &'static str> {
            ACQUIRING.with(|active| {
                if active.replace(true) {
                    Err("GPU acquisition pending")
                } else {
                    Ok(Self)
                }
            })
        }
    }
    impl Drop for Acquisition {
        fn drop(&mut self) {
            ACQUIRING.with(|active| active.set(false));
        }
    }
    async fn delay(ms: i32) {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            if let Some(window) = web_sys::window() {
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
            }
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
    pub struct Renderer {
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
        pipeline: wgpu::RenderPipeline,
        canvas: web_sys::HtmlCanvasElement,
        pub failed: Arc<AtomicBool>,
        pub quality: u32,
    }
    impl Drop for Renderer {
        fn drop(&mut self) {
            self.device.destroy();
        }
    }
    impl Renderer {
        pub async fn acquire(canvas: web_sys::HtmlCanvasElement) -> Result<Self, &'static str> {
            // No browser entry point or future construction precedes admission.
            let acquisition = Acquisition::take()?;
            if let Some(performance) = web_sys::window().and_then(|w| w.performance()) {
                canvas
                    .set_attribute("data-acquisition-start-ms", &performance.now().to_string())
                    .ok();
            }
            let creating = Box::pin(Self::new(canvas));
            match futures::future::select(creating, Box::pin(delay(700))).await {
                futures::future::Either::Left((result, _)) => result,
                futures::future::Either::Right((_, creating)) => {
                    // Do not abandon a requestDevice promise: a late acquired device
                    // must still be explicitly destroyed when its renderer drops.
                    wasm_bindgen_futures::spawn_local(async move {
                        // Destroy can synchronously reenter the mounted UI. Keep
                        // admission held until that callback has also returned.
                        drop(creating.await);
                        drop(acquisition);
                    });
                    Err("GPU acquisition timed out")
                }
            }
        }
        async fn new(canvas: web_sys::HtmlCanvasElement) -> Result<Self, &'static str> {
            let gpu = js_sys::Reflect::get(
                &js_sys::Reflect::get(&js_sys::global(), &"navigator".into())
                    .map_err(|_| "GPU API unavailable")?,
                &"gpu".into(),
            )
            .map_err(|_| "GPU API unavailable")?;
            if gpu.is_undefined() || gpu.is_null() {
                return Err("GPU API unavailable");
            }
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let surface = instance
                .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
                .map_err(|_| "GPU canvas unavailable")?;
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    compatible_surface: Some(&surface),
                    ..Default::default()
                })
                .await
                .map_err(|_| "GPU adapter unavailable")?;
            if adapter.limits().max_texture_dimension_2d < 1 {
                return Err("GPU adapter limits unavailable");
            }
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default())
                .await
                .map_err(|_| "GPU device unavailable")?;
            let failed = Arc::new(AtomicBool::new(false));
            let lost = failed.clone();
            device.set_device_lost_callback(move |_, _| {
                lost.store(true, Ordering::Relaxed);
            });
            let errors = failed.clone();
            device.on_uncaptured_error(Arc::new(move |_| {
                errors.store(true, Ordering::Relaxed);
            }));
            let Some(config) = surface.get_default_config(&adapter, 1, 1) else {
                device.destroy();
                return Err("GPU surface format unavailable");
            };
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Verified derivative integer presentation"),
                source: wgpu::ShaderSource::Wgsl(include_str!("presentation.wgsl").into()),
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Verified derivative"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fragment"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
            if scope.pop().await.is_some() {
                device.destroy();
                return Err("GPU pipeline unavailable");
            }
            Ok(Self {
                device,
                queue,
                surface,
                config,
                pipeline,
                canvas,
                failed,
                quality: 512,
            })
        }
        pub fn is_failed(&self) -> bool {
            self.failed.load(Ordering::Relaxed)
        }
        pub async fn image(
            &mut self,
            image: &web_sys::HtmlImageElement,
            orientation: u8,
        ) -> Result<Allocation, &'static str> {
            if self.is_failed() {
                return Err("GPU device lost");
            }
            if !(1..=8).contains(&orientation) {
                return Err("GPU orientation unavailable");
            }
            let input = allocation(
                image.natural_width(),
                image.natural_height(),
                self.device.limits().max_texture_dimension_2d,
                1024,
            )?;
            let (ow, oh) = if orientation >= 5 {
                (image.natural_height(), image.natural_width())
            } else {
                (image.natural_width(), image.natural_height())
            };
            let budget = allocation(
                ow,
                oh,
                self.device.limits().max_texture_dimension_2d,
                self.quality,
            )?;
            // Downsample before ImageData / WASM staging or GPU allocation.
            let scratch = web_sys::window()
                .and_then(|w| w.document())
                .ok_or("Decode canvas unavailable")?
                .create_element("canvas")
                .map_err(|_| "Decode canvas unavailable")?
                .dyn_into::<web_sys::HtmlCanvasElement>()
                .map_err(|_| "Decode canvas unavailable")?;
            scratch.set_width(input.width);
            scratch.set_height(input.height);
            let context = scratch
                .get_context("2d")
                .map_err(|_| "Decode canvas unavailable")?
                .ok_or("Decode canvas unavailable")?
                .dyn_into::<web_sys::CanvasRenderingContext2d>()
                .map_err(|_| "Decode canvas unavailable")?;
            context.set_image_smoothing_enabled(false);
            context
                .draw_image_with_html_image_element_and_dw_and_dh(
                    image,
                    0.0,
                    0.0,
                    input.width as f64,
                    input.height as f64,
                )
                .map_err(|_| "GPU staging unavailable")?;
            let pixels = context
                .get_image_data(0, 0, input.width as i32, input.height as i32)
                .map_err(|_| "GPU staging unavailable")?
                .data()
                .0;
            scratch.set_width(1);
            scratch.set_height(1);
            let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let extent = wgpu::Extent3d {
                width: input.width,
                height: input.height,
                depth_or_array_layers: 1,
            };
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Verified RGBA upload"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(input.width * 4),
                    rows_per_image: Some(input.height),
                },
                extent,
            );
            drop(pixels);
            self.canvas.set_width(budget.width);
            self.canvas.set_height(budget.height);
            self.config.width = budget.width;
            self.config.height = budget.height;
            self.surface.configure(&self.device, &self.config);
            let frame = match self.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame) => frame,
                _ => {
                    texture.destroy();
                    scope.pop().await;
                    return Err("GPU surface unavailable");
                }
            };
            let view = texture.create_view(&Default::default());
            let params = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut values = Vec::with_capacity(16);
            for value in [u32::from(orientation), budget.width, budget.height, 0] {
                values.extend_from_slice(&value.to_le_bytes());
            }
            self.queue.write_buffer(&params, 0, &values);
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: params.as_entire_binding(),
                    },
                ],
            });
            let target = frame.texture.create_view(&Default::default());
            let mut encoder = self.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.draw(0..3, 0..1);
            }
            let submitted = web_sys::window()
                .and_then(|w| w.performance())
                .map(|p| p.now())
                .unwrap_or(0.0);
            if let Some(performance) = web_sys::window().and_then(|w| w.performance()) {
                let _ = performance.mark(&format!("lytronaut-gpu-submit-{submitted}"));
            }
            self.queue.submit([encoder.finish()]);
            self.queue.present(frame);
            self.canvas
                .set_attribute("data-submitted-ms", &submitted.to_string())
                .ok();
            let validation =
                futures::future::select(Box::pin(scope.pop()), Box::pin(delay(100))).await;
            let invalid = match validation {
                futures::future::Either::Left((error, _)) => error.is_some(),
                futures::future::Either::Right(_) => true,
            };
            if invalid || self.is_failed() {
                texture.destroy();
                params.destroy();
                return Err("GPU presentation failed");
            }
            texture.destroy();
            params.destroy();
            self.canvas
                .set_attribute("data-cpu-bytes", &budget.cpu_bytes.to_string())
                .ok();
            self.canvas
                .set_attribute("data-gpu-bytes", &budget.gpu_bytes.to_string())
                .ok();
            Ok(budget)
        }
    }
}
#[cfg(target_arch = "wasm32")]
pub use web::Renderer;
#[cfg(all(target_arch = "wasm32", feature = "test-harness"))]
pub(crate) use web::acquisition_pending;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_and_aspect() {
        assert_eq!(allocation(2048, 1024, 16384, 1024).unwrap().width, 1024);
        assert_eq!(allocation(2048, 1024, 16384, 1024).unwrap().height, 512);
        for (w, h) in [(0, 1), (1, 0), (2049, 1), (1, 2049), (u32::MAX, u32::MAX)] {
            assert!(allocation(w, h, 16384, 1024).is_err());
        }
        assert!(allocation(1, 1, 0, 1024).is_err());
        assert!(allocation(1, 1, 1024, 0).is_err());
        for w in [1, 127, 512, 1024, 2048] {
            for h in [1, 511, 1024, 2048] {
                for cap in [1, 512, 1024, 16384] {
                    let a = allocation(w, h, cap, 1024).unwrap();
                    assert!(
                        a.width <= 1024 && a.height <= 1024 && a.width <= cap && a.height <= cap
                    );
                    assert!(a.cpu_bytes <= 96 * 1024 * 1024 && a.gpu_bytes <= 64 * 1024 * 1024);
                }
            }
        }
        assert_eq!(allocation(1024, 1024, 16384, 512).unwrap().width, 512);
        let focus = allocation(512, 512, 16384, 512).unwrap();
        assert!(focus.cpu_bytes - 32 * 1024 * 1024 <= 24 * 1024 * 1024);
    }
}

use crate::appview::AppView;
use crate::product::{Gpu, Pointer, PointerKind, Product, WindowSpec};

use std::sync::Arc;

use winit::{
    application::ApplicationHandler, event::*, event_loop::{ActiveEventLoop, EventLoop}, keyboard::{KeyCode, PhysicalKey}, window::{Window, WindowId}
};

/// GPU とウィンドウができたあとに製品を作る関数
pub type MakeProduct<P> = Box<dyn FnOnce(&Gpu, &[WindowId]) -> anyhow::Result<P>>;

pub struct App<P: Product> {
    #[cfg(target_arch = "wasm32")]
    proxy: Option<winit::event_loop::EventLoopProxy<AppView>>,
    windows: Vec<WindowSpec>,
    make_product: Option<MakeProduct<P>>,
    state: Option<(AppView, P)>,
}

impl<P: Product> App<P> {
    pub fn new(#[cfg(target_arch = "wasm32")] event_loop: &EventLoop<AppView>, windows: Vec<WindowSpec>, make_product: MakeProduct<P>) -> Self {
        #[cfg(target_arch = "wasm32")]
        let proxy = Some(event_loop.create_proxy());
        Self {
            state: None,
            windows,
            make_product: Some(make_product),
            #[cfg(target_arch = "wasm32")]
            proxy,
        }
    }

    fn start(&mut self, appview: AppView) {
        let Some(make_product) = self.make_product.take() else {
            return;
        };
        match make_product(&appview.gpu, &appview.window_ids()) {
            Ok(product) => self.state = Some((appview, product)),
            Err(e) => log::error!("could not start: {:#}", e),
        }
    }
}

fn attributes(spec: &WindowSpec) -> winit::window::WindowAttributes {
    let mut attributes = Window::default_attributes().with_title(spec.title.clone());
    if let Some([w, h]) = spec.size {
        attributes = attributes.with_inner_size(winit::dpi::LogicalSize::new(w, h));
    }
    attributes
}

impl<P: Product> ApplicationHandler<AppView> for App<P> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let windows = self.windows.iter()
                .map(|spec| Arc::new(event_loop.create_window(attributes(spec)).unwrap()))
                .collect();
            match pollster::block_on(AppView::new(windows)) {
                Ok(appview) => self.start(appview),
                Err(e) => log::error!("could not create GPU resources: {:#}", e),
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            use winit::platform::web::WindowAttributesExtWebSys;

            const CANVAS_ID: &str = "canvas";

            let window = wgpu::web_sys::window().unwrap();
            let document = window.document().unwrap();
            let canvas = document.get_element_by_id(CANVAS_ID).unwrap();
            let html_canvas_element = canvas.unchecked_into();
            let window_attributes = attributes(&self.windows[0]).with_canvas(Some(html_canvas_element));
            let window = Arc::new(event_loop.create_window(window_attributes).unwrap());

            // Run the future asynchronously and use the
            // proxy to send the results to the event loop
            if let Some(proxy) = self.proxy.take() {
                wasm_bindgen_futures::spawn_local(async move {
                    assert!(proxy
                        .send_event(
                            AppView::new(vec![window])
                                .await
                                .expect("Unable to create canvas!!!")
                        )
                        .is_ok())
                });
            }
        }
    }

    #[allow(unused_mut)]
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, mut event: AppView) {
        // This is where proxy.send_event() ends up
        #[cfg(target_arch = "wasm32")]
        {
            let device = &event.gpu.device;
            let window = &mut event.windows[0];
            window.nature.request_redraw();
            window.resize(
                window.nature.inner_size().width,
                window.nature.inner_size().height,
                device
            );
        }
        self.start(event);
    }

     fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {

        let Some((appview, product)) = &mut self.state else {
            return;
        };
        // find window index first (immutable borrow) to avoid overlapping mutable borrows
        let window_index = match appview.windows.iter().position(|w| w.nature.id() == window_id) {
            Some(i) => i,
            None => return,
        };
        match event {
            WindowEvent::CloseRequested => {
                appview.close(window_id);
                if product.window_closed(window_id) || appview.windows.is_empty() {
                    event_loop.exit()
                }
                return
            },
            WindowEvent::Resized(size) => {
                let window_wrapper = &mut appview.windows[window_index];
                window_wrapper.resize(size.width, size.height, &appview.gpu.device)
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state: key_state,
                        repeat,
                        ..
                    },
                ..
            } => {
                match (code, key_state.is_pressed()) {
                    (KeyCode::Escape, true) => event_loop.exit(),
                    (KeyCode::KeyR, true) => {
                        appview.reload();
                        product.reload(&appview.gpu);
                    },
                    _ => {}
                }
                let window_wrapper = &mut appview.windows[window_index];
                window_wrapper.handle_key(product, code, key_state.is_pressed(), repeat);
            },
            WindowEvent::Focused(false) => {
                appview.windows[window_index].focus_lost(product);
            }
            WindowEvent::RedrawRequested => {
                let window_wrapper = &mut appview.windows[window_index];
                window_wrapper.update(&appview.gpu, &mut appview.text_system, product);
                match window_wrapper.render(&appview.gpu, &appview.ui_renderer, product) {
                    Ok(_) => {}
                    // Reconfigure the surface if it's lost or outdated
                    Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                        let size = window_wrapper.nature.inner_size();
                        window_wrapper.resize(size.width, size.height, &appview.gpu.device);
                    }
                    Err(e) => {
                        log::error!("Unable to render {}", e);
                    }
                }
            },
            WindowEvent::CursorMoved { position, .. } => {
                let window_wrapper = &mut appview.windows[window_index];
                window_wrapper.cursor_moved(position);
                pointer(product, window_wrapper, PointerKind::Moved);
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                let kind = if state.is_pressed() { PointerKind::Down } else { PointerKind::Up };
                pointer(product, &appview.windows[window_index], kind);
            }
            _ => {}
        }
    }
}

fn pointer(product: &mut dyn Product, window: &crate::window::WindowWrapper, kind: PointerKind) {
    if let Some(position) = window.logical_cursor() {
        product.pointer(Pointer { window: window.nature.id(), position, window_size: window.logical_size(), kind });
    }
}

/// 製品を起動する。`make_product` は GPU と `windows` の各ウィンドウができたあとに 1 回呼ばれる
pub fn run<P: Product>(windows: Vec<WindowSpec>, make_product: impl FnOnce(&Gpu, &[WindowId]) -> anyhow::Result<P> + 'static) -> anyhow::Result<()> {
    let event_loop = EventLoop::with_user_event().build()?;
    let mut app = App::new(
        #[cfg(target_arch = "wasm32")]
        &event_loop,
        windows,
        Box::new(make_product),
    );
    event_loop.run_app(&mut app)?;
    Ok(())
}

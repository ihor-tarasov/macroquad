//!
//! `macroquad` is a simple and easy to use game library for Rust programming language.
//!
//! `macroquad` attempts to avoid any rust-specific programming concepts like lifetimes/borrowing, making it very friendly for rust beginners.
//!
//! ## Supported platforms
//!
//! * PC: Windows/Linux/MacOS
//! * HTML5
//! * Android
//! * IOS
//!
//! ## Features
//!
//! * Same code for all supported platforms, no platform dependent defines required
//! * Efficient 2D rendering with automatic geometry batching
//! * Minimal amount of dependencies: build after `cargo clean` takes only 16s on x230(~6years old laptop)
//! * Immediate mode UI library included
//! * Single command deploy for both WASM and Android [build instructions](https://github.com/not-fl3/miniquad/#building-examples)
//! # Example
//! ```no_run
//! use macroquad::prelude::*;
//!
//! #[macroquad::main("BasicShapes")]
//! async fn main() {
//!     loop {
//!         clear_background(RED);
//!
//!         draw_line(40.0, 40.0, 100.0, 200.0, 15.0, BLUE);
//!         draw_rectangle(screen_width() / 2.0 - 60.0, 100.0, 120.0, 60.0, GREEN);
//!         draw_circle(screen_width() - 30.0, screen_height() - 30.0, 15.0, YELLOW);
//!         draw_text("HELLO", 20.0, 20.0, 20.0, DARKGRAY);
//!
//!         next_frame().await
//!     }
//! }
//!```
#![allow(warnings)]
use miniquad::*;

use slotmap::SlotMap;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;

mod exec;
mod quad_gl;

pub mod audio;
pub mod camera;
pub mod color;
pub mod file;
pub mod input;
pub mod material;
pub mod math;
pub mod models;
pub mod shapes;
pub mod text;
pub mod texture;
pub mod time;
pub mod ui;
pub mod window;

pub mod experimental;

pub mod prelude;

pub mod telemetry;

mod cubemap;
mod error;

pub use error::Error;

pub mod scene_graph;

/// Cross platform random generator.
pub mod rand {
    pub use quad_rand::*;
}

#[cfg(not(feature = "log-rs"))]
/// Logging macros, available with miniquad "log-impl" feature.
pub mod logging {
    pub use miniquad::{debug, error, info, trace, warn};
}
#[cfg(feature = "log-rs")]
// Use logging facade
pub use ::log as logging;
pub use miniquad;

use crate::{
    color::{colors::*, Color},
    quad_gl::QuadGl,
    texture::TextureHandle,
    ui::ui_context::UiContext,
};

use glam::{vec2, Mat4, Vec2};
use std::sync::{Arc, Mutex, Weak};

pub(crate) mod thread_assert {
    static mut THREAD_ID: Option<std::thread::ThreadId> = None;

    pub fn set_thread_id() {
        unsafe {
            THREAD_ID = Some(std::thread::current().id());
        }
    }

    pub fn same_thread() {
        unsafe {
            let thread_id = std::thread::current().id();
            assert!(THREAD_ID.is_some());
            assert!(THREAD_ID.unwrap() == thread_id);
        }
    }
}
struct Context {
    audio_context: audio::AudioContext,

    screen_width: f32,
    screen_height: f32,

    simulate_mouse_with_touch: bool,

    keys_down: HashSet<KeyCode>,
    keys_pressed: HashSet<KeyCode>,
    keys_released: HashSet<KeyCode>,
    mouse_down: HashSet<MouseButton>,
    mouse_pressed: HashSet<MouseButton>,
    mouse_released: HashSet<MouseButton>,
    touches: HashMap<u64, input::Touch>,
    chars_pressed_queue: Vec<char>,
    chars_pressed_ui_queue: Vec<char>,
    mouse_position: Vec2,
    last_mouse_position: Option<Vec2>,
    mouse_raw_delta: Vec2,
    mouse_wheel: Vec2,

    prevent_quit_event: bool,
    quit_requested: bool,

    cursor_grabbed: bool,

    input_events: Vec<Vec<MiniquadInputEvent>>,

    camera_matrix: Option<Mat4>,

    // ui_context: UiContext,
    coroutines_context: experimental::coroutines::CoroutinesContext,

    start_time: f64,
    last_frame_time: f64,
    frame_time: f64,

    #[cfg(one_screenshot)]
    counter: usize,

    //texture_batcher: texture::Batcher,
    unwind: bool,
    recovery_future: Option<Pin<Box<dyn Future<Output = ()>>>>,

    //quad_context: Box<dyn miniquad::RenderingBackend>,
    textures: crate::texture::TexturesContext,
}

#[derive(Clone)]
enum MiniquadInputEvent {
    MouseMotion {
        x: f32,
        y: f32,
    },
    MouseWheel {
        x: f32,
        y: f32,
    },
    MouseButtonDown {
        x: f32,
        y: f32,
        btn: MouseButton,
    },
    MouseButtonUp {
        x: f32,
        y: f32,
        btn: MouseButton,
    },
    Char {
        character: char,
        modifiers: KeyMods,
        repeat: bool,
    },
    KeyDown {
        keycode: KeyCode,
        modifiers: KeyMods,
        repeat: bool,
    },
    KeyUp {
        keycode: KeyCode,
        modifiers: KeyMods,
    },
    Touch {
        phase: TouchPhase,
        id: u64,
        x: f32,
        y: f32,
    },
}

impl MiniquadInputEvent {
    fn repeat<T: miniquad::EventHandler>(&self, t: &mut T) {
        use crate::MiniquadInputEvent::*;
        match self {
            MouseMotion { x, y } => t.mouse_motion_event(*x, *y),
            MouseWheel { x, y } => t.mouse_wheel_event(*x, *y),
            MouseButtonDown { x, y, btn } => t.mouse_button_down_event(*btn, *x, *y),
            MouseButtonUp { x, y, btn } => t.mouse_button_up_event(*btn, *x, *y),
            Char {
                character,
                modifiers,
                repeat,
            } => t.char_event(*character, *modifiers, *repeat),
            KeyDown {
                keycode,
                modifiers,
                repeat,
            } => t.key_down_event(*keycode, *modifiers, *repeat),
            KeyUp { keycode, modifiers } => t.key_up_event(*keycode, *modifiers),
            Touch { phase, id, x, y } => t.touch_event(*phase, *id, *x, *y),
        }
    }
}

impl Context {
    //const DEFAULT_BG_COLOR: Color = BLACK;

    fn new() -> Context {
        let (screen_width, screen_height) = miniquad::window::screen_size();

        Context {
            screen_width,
            screen_height,

            simulate_mouse_with_touch: true,

            keys_down: HashSet::new(),
            keys_pressed: HashSet::new(),
            keys_released: HashSet::new(),
            chars_pressed_queue: Vec::new(),
            chars_pressed_ui_queue: Vec::new(),
            mouse_down: HashSet::new(),
            mouse_pressed: HashSet::new(),
            mouse_released: HashSet::new(),
            touches: HashMap::new(),
            mouse_position: vec2(0., 0.),
            last_mouse_position: None,
            mouse_raw_delta: vec2(0., 0.),
            mouse_wheel: vec2(0., 0.),

            prevent_quit_event: false,
            quit_requested: false,

            cursor_grabbed: false,

            input_events: Vec::new(),

            camera_matrix: None,

            // ui_context: UiContext::new(&mut *ctx, screen_width, screen_height),
            // fonts_storage: text::FontsStorage::new(&mut *ctx),
            // texture_batcher: texture::Batcher::new(&mut *ctx),
            audio_context: audio::AudioContext::new(),
            coroutines_context: experimental::coroutines::CoroutinesContext::new(),

            start_time: miniquad::date::now(),
            last_frame_time: miniquad::date::now(),
            frame_time: 1. / 60.,

            #[cfg(one_screenshot)]
            counter: 0,
            unwind: false,
            recovery_future: None,

            textures: crate::texture::TexturesContext::new(),
        }
    }

    /// Returns the handle for this texture.
    pub fn raw_miniquad_id(&self, handle: &TextureHandle) -> miniquad::TextureId {
        // match handle {
        //     TextureHandle::Unmanaged(texture) => *texture,
        //     TextureHandle::Managed(texture) => self
        //         .textures
        //         .texture(texture.0)
        //         .unwrap_or(self.white_texture),
        //     TextureHandle::ManagedWeak(texture) => self
        //         .textures
        //         .texture(*texture)
        //         .unwrap_or(self.white_texture),
        // }
        unimplemented!()
    }

    fn end_frame(&mut self) {
        //crate::experimental::scene::update();

        //self.perform_render_passes();

        // self.ui_context.draw(get_quad_context(), &mut self.gl);
        // let screen_mat = self.pixel_perfect_projection_matrix();
        // self.gl.draw(get_quad_context(), screen_mat);

        //for canvas in self.scene_graph.canvases {}

        get_quad_context().commit_frame();

        #[cfg(one_screenshot)]
        {
            get_context().counter += 1;
            if get_context().counter == 3 {
                crate::prelude::get_screen_data().export_png("screenshot.png");
                panic!("screenshot successfully saved to `screenshot.png`");
            }
        }

        telemetry::end_gpu_query();

        self.mouse_wheel = Vec2::new(0., 0.);
        self.keys_pressed.clear();
        self.keys_released.clear();
        self.mouse_pressed.clear();
        self.mouse_released.clear();

        self.quit_requested = false;

        // remove all touches that were Ended or Cancelled
        self.touches.retain(|_, touch| {
            touch.phase != input::TouchPhase::Ended && touch.phase != input::TouchPhase::Cancelled
        });

        // change all Started or Moved touches to Stationary
        for touch in self.touches.values_mut() {
            if touch.phase == input::TouchPhase::Started || touch.phase == input::TouchPhase::Moved
            {
                touch.phase = input::TouchPhase::Stationary;
            }
        }
    }

    pub(crate) fn pixel_perfect_projection_matrix(&self) -> glam::Mat4 {
        let (width, height) = miniquad::window::screen_size();

        let dpi = miniquad::window::dpi_scale();

        glam::Mat4::orthographic_rh_gl(0., width / dpi, height / dpi, 0., -1., 1.)
    }

    pub(crate) fn projection_matrix(&self) -> glam::Mat4 {
        if let Some(matrix) = self.camera_matrix {
            matrix
        } else {
            self.pixel_perfect_projection_matrix()
        }
    }
}

#[no_mangle]
static mut CONTEXT: Option<Context> = None;

// This is required for #[macroquad::test]
//
// unfortunately #[cfg(test)] do not work with integration tests
// so this module should be publicly available
#[doc(hidden)]
pub mod test {
    pub static mut MUTEX: Option<std::sync::Mutex<()>> = None;
    pub static ONCE: std::sync::Once = std::sync::Once::new();
}

fn get_context() -> &'static mut Context {
    thread_assert::same_thread();

    unsafe { CONTEXT.as_mut().unwrap_or_else(|| panic!()) }
}

fn get_quad_context() -> &'static mut dyn miniquad::RenderingBackend {
    unimplemented!()
}

struct Stage {
    main_future: Option<Pin<Box<dyn Future<Output = ()>>>>,
    ctx: Arc<Context2>,
}

impl EventHandler for Stage {
    fn resize_event(&mut self, width: f32, height: f32) {
        let _z = telemetry::ZoneGuard::new("Event::resize_event");
        // get_context().screen_width = width;
        // get_context().screen_height = height;
    }

    fn raw_mouse_motion(&mut self, x: f32, y: f32) {
        let context = get_context();

        context.mouse_raw_delta = vec2(x, y);
        // if context.cursor_grabbed {
        //     //context.mouse_position += Vec2::new(x, y);

        //     let event = MiniquadInputEvent::MouseMotion {
        //         x: context.mouse_position.x,
        //         y: context.mouse_position.y,
        //     };
        //     context
        //         .input_events
        //         .iter_mut()
        //         .for_each(|arr| arr.push(event.clone()));
        // }
    }

    fn mouse_motion_event(&mut self, x: f32, y: f32) {
        let context = get_context();

        if !context.cursor_grabbed {
            context.mouse_position = Vec2::new(x, y);

            context
                .input_events
                .iter_mut()
                .for_each(|arr| arr.push(MiniquadInputEvent::MouseMotion { x, y }));
        }
    }

    fn mouse_wheel_event(&mut self, x: f32, y: f32) {
        let context = get_context();

        context.mouse_wheel.x = x;
        context.mouse_wheel.y = y;

        context
            .input_events
            .iter_mut()
            .for_each(|arr| arr.push(MiniquadInputEvent::MouseWheel { x, y }));
    }

    fn mouse_button_down_event(&mut self, btn: MouseButton, x: f32, y: f32) {
        let context = get_context();

        context.mouse_down.insert(btn);
        context.mouse_pressed.insert(btn);

        context
            .input_events
            .iter_mut()
            .for_each(|arr| arr.push(MiniquadInputEvent::MouseButtonDown { x, y, btn }));

        if !context.cursor_grabbed {
            context.mouse_position = Vec2::new(x, y);
        }
    }

    fn mouse_button_up_event(&mut self, btn: MouseButton, x: f32, y: f32) {
        let context = get_context();

        context.mouse_down.remove(&btn);
        context.mouse_released.insert(btn);

        context
            .input_events
            .iter_mut()
            .for_each(|arr| arr.push(MiniquadInputEvent::MouseButtonUp { x, y, btn }));

        if !context.cursor_grabbed {
            context.mouse_position = Vec2::new(x, y);
        }
    }

    fn touch_event(&mut self, phase: TouchPhase, id: u64, x: f32, y: f32) {
        let context = get_context();

        context.touches.insert(
            id,
            input::Touch {
                id,
                phase: phase.into(),
                position: Vec2::new(x, y),
            },
        );

        if context.simulate_mouse_with_touch {
            if phase == TouchPhase::Started {
                self.mouse_button_down_event(MouseButton::Left, x, y);
            }

            if phase == TouchPhase::Ended {
                self.mouse_button_up_event(MouseButton::Left, x, y);
            }

            if phase == TouchPhase::Moved {
                self.mouse_motion_event(x, y);
            }
        };

        context
            .input_events
            .iter_mut()
            .for_each(|arr| arr.push(MiniquadInputEvent::Touch { phase, id, x, y }));
    }

    fn char_event(&mut self, character: char, modifiers: KeyMods, repeat: bool) {
        let context = get_context();

        context.chars_pressed_queue.push(character);
        context.chars_pressed_ui_queue.push(character);

        context.input_events.iter_mut().for_each(|arr| {
            arr.push(MiniquadInputEvent::Char {
                character,
                modifiers,
                repeat,
            })
        });
    }

    fn key_down_event(&mut self, keycode: KeyCode, modifiers: KeyMods, repeat: bool) {
        let context = get_context();
        context.keys_down.insert(keycode);
        if repeat == false {
            context.keys_pressed.insert(keycode);
        }

        context.input_events.iter_mut().for_each(|arr| {
            arr.push(MiniquadInputEvent::KeyDown {
                keycode,
                modifiers,
                repeat,
            })
        });
    }

    fn key_up_event(&mut self, keycode: KeyCode, modifiers: KeyMods) {
        let context = get_context();
        context.keys_down.remove(&keycode);
        context.keys_released.insert(keycode);

        context
            .input_events
            .iter_mut()
            .for_each(|arr| arr.push(MiniquadInputEvent::KeyUp { keycode, modifiers }));
    }

    fn update(&mut self) {
        let _z = telemetry::ZoneGuard::new("Event::update");

        // Unless called every frame, cursor will not remain grabbed
        //miniquad::window::set_cursor_grab(get_context().cursor_grabbed);

        #[cfg(not(target_arch = "wasm32"))]
        {
            // TODO: consider making it a part of miniquad?
            std::thread::yield_now();
        }
    }

    fn draw(&mut self) {
        {
            let _z = telemetry::ZoneGuard::new("Event::draw");

            use std::panic;

            let scene = scene_graph::Scene {
                data: &self.ctx.scene,
                ctx: self.ctx.clone(),
            };
            {
                let _z = telemetry::ZoneGuard::new("Event::draw begin_frame");

                scene.clear(Color::new(0.2, 0.2, 0.5, 1.));
            }

            fn maybe_unwind(unwind: bool, f: impl FnOnce() + Sized + panic::UnwindSafe) -> bool {
                if unwind {
                    panic::catch_unwind(|| f()).is_ok()
                } else {
                    f();
                    true
                }
            }

            //let result = maybe_unwind(get_context().unwind, || {
            if let Some(future) = self.main_future.as_mut() {
                let _z = telemetry::ZoneGuard::new("Event::draw user code");

                if exec::resume(future).is_some() {
                    self.main_future = None;
                    miniquad::window::quit();
                    return;
                }
                //get_context().coroutines_context.update();
            }
            //});

            scene.draw_canvas(0);
            for camera in &mut *scene.data.cameras.lock() {
                let (proj, view) = camera.proj_view();
                if let crate::camera::Environment::Skybox(ref mut cubemap) = camera.environment {
                    cubemap.draw(&mut **scene.data.quad_context.lock(), &proj, &view);
                }
                for (model, t) in &mut *scene.data.models.lock() {
                    let mat = t.matrix();
                    scene.draw_model(model, camera, mat);
                }
            }
            scene.draw_canvas(1);
            // if result == false {
            //     if let Some(recovery_future) = get_context().recovery_future.take() {
            //         self.main_future = Some(recovery_future);
            //     }
            // }

            {
                let _z = telemetry::ZoneGuard::new("Event::draw end_frame");
                //get_context().end_frame();
                let mut ctx = scene.data.quad_context.lock();
                ctx.commit_frame()
            }
            // get_context().frame_time = date::now() - get_context().last_frame_time;
            // get_context().last_frame_time = date::now();

            #[cfg(any(target_arch = "wasm32", target_os = "linux"))]
            {
                let _z = telemetry::ZoneGuard::new("glFinish/glFLush");

                unsafe {
                    miniquad::gl::glFlush();
                    miniquad::gl::glFinish();
                }
            }
        }

        //telemetry::reset();
    }

    fn window_restored_event(&mut self) {
        #[cfg(target_os = "android")]
        get_context().audio_context.resume();
    }

    fn window_minimized_event(&mut self) {
        #[cfg(target_os = "android")]
        get_context().audio_context.pause();
    }

    fn quit_requested_event(&mut self) {
        let context = get_context();
        if context.prevent_quit_event {
            miniquad::window::cancel_quit();
            context.quit_requested = true;
        }
    }
}

pub struct Context2 {
    scene: scene_graph::SceneData,
}

pub struct Context3 {
    ctx: Arc<Context2>,
}

impl Context2 {
    pub(crate) fn new() -> Context2 {
        let mut ctx: Box<dyn miniquad::RenderingBackend> =
            miniquad::window::new_rendering_backend();
        let scene = scene_graph::SceneData::new(ctx);

        Context2 { scene }
    }
}

impl Context3 {
    pub fn scene(&self) -> scene_graph::Scene {
        scene_graph::Scene {
            data: &self.ctx.scene,
            ctx: self.ctx.clone(),
        }
    }

    pub fn f(&mut self) {
        println!("waaaaaat");
    }
}

pub fn start<F: Fn(Context3) -> Fut + 'static, Fut: Future<Output = ()> + 'static>(
    mut config: conf::Conf,
    future: F,
) {
    miniquad::start(conf::Conf { ..config }, move || {
        thread_assert::set_thread_id();
        unsafe { CONTEXT = Some(Context::new()) };
        let ctx = Context3 {
            ctx: Arc::new(Context2::new()),
        };
        Box::new(Stage {
            main_future: Some(Box::pin(future(Context3 {
                ctx: Arc::clone(&ctx.ctx),
            }))),
            ctx: ctx.ctx,
        })
    });
}

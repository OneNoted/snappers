use std::num::NonZeroU32;

use anyhow::{Context, Result};
use cairo::{Context as CairoContext, Format, ImageSurface};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm, delegate_touch,
    output::{OutputHandler, OutputInfo, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Modifiers, RawModifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        touch::TouchHandler,
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface, wl_touch},
};

use crate::{
    capture::CaptureSnapshot,
    config::{AppConfig, KeyBinding},
    geometry::{Point, Rect, Size},
    render::{
        PanelAssets, PixelSurface, build_panel_assets, capture_button_hit, paint_background,
        paint_masks_and_border, paint_panel,
    },
    state::{ButtonState, OutputState as ModelOutputState, PointerUpOutcome, SelectionModel},
};

#[derive(Debug, Clone)]
pub struct OverlayResult {
    pub region: Rect,
    pub show_pointer: bool,
    pub write_to_disk: bool,
}

pub fn select_region(
    snapshot: CaptureSnapshot,
    config: AppConfig,
    show_pointer: bool,
) -> Result<Option<OverlayResult>> {
    let conn =
        Connection::connect_to_env().context("failed to connect to wayland for the overlay UI")?;
    let (globals, mut event_queue) = registry_queue_init(&conn)?;
    let qh = event_queue.handle();

    let compositor =
        CompositorState::bind(&globals, &qh).context("wl_compositor is not available")?;
    let layer_shell =
        LayerShell::bind(&globals, &qh).context("wlr-layer-shell is not available")?;
    let shm = Shm::bind(&globals, &qh).context("wl_shm is not available")?;

    let mut app = OverlayApp {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        panels: build_panel_assets()?,
        overlays: Vec::new(),
        model: None,
        keyboard: None,
        pointer: None,
        touch: None,
        modifiers: Modifiers::default(),
        exit: None,
        snapshot,
        config,
    };

    event_queue.roundtrip(&mut app)?;

    let known_outputs: Vec<_> = app
        .output_state
        .outputs()
        .filter_map(|output| app.output_state.info(&output).map(|info| (output, info)))
        .collect();

    let mut matched = Vec::new();
    for capture in &app.snapshot.outputs {
        let Some((wl_output, info)) = known_outputs
            .iter()
            .find(|(_, info)| info.name.as_deref() == Some(capture.name.as_str()))
        else {
            continue;
        };

        let surface = compositor.create_surface(&qh);
        let layer = layer_shell.create_layer_surface(
            &qh,
            surface,
            Layer::Overlay,
            Some("snappers"),
            Some(wl_output),
        );
        layer.set_anchor(Anchor::all());
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.commit();

        let size = logical_size(info)
            .with_context(|| format!("output {} is missing logical size", capture.name))?;
        matched.push(OverlaySurface {
            logical_size: size,
            layer,
            pool: SlotPool::new((size.width * size.height * 4) as usize, &app.shm)
                .context("failed to create shm pool")?,
            with_pointer: PixelSurface::from_rgba_image(&capture.screenshot_with_pointer),
            without_pointer: PixelSurface::from_rgba_image(&capture.screenshot_without_pointer),
            configured: false,
        });
    }

    if matched.is_empty() {
        anyhow::bail!(
            "the overlay could not match any captured outputs to current wayland outputs"
        );
    }

    let model_outputs = matched
        .iter()
        .map(|surface| ModelOutputState {
            logical_rect: Rect::new(
                0,
                0,
                surface.logical_size.width,
                surface.logical_size.height,
            ),
        })
        .collect::<Vec<_>>();
    app.model = Some(SelectionModel::new(model_outputs, 0, show_pointer));
    app.overlays = matched;

    while app.exit.is_none() {
        event_queue.blocking_dispatch(&mut app)?;
        if app.overlays.iter().all(|surface| surface.configured) {
            app.draw_all(&qh)?;
        }
    }

    Ok(app.exit.unwrap_or(None))
}

struct OverlaySurface {
    logical_size: Size,
    layer: LayerSurface,
    pool: SlotPool,
    with_pointer: PixelSurface,
    without_pointer: PixelSurface,
    configured: bool,
}

struct OverlayApp {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    panels: PanelAssets,
    overlays: Vec<OverlaySurface>,
    model: Option<SelectionModel>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    touch: Option<wl_touch::WlTouch>,
    modifiers: Modifiers,
    exit: Option<Option<OverlayResult>>,
    snapshot: CaptureSnapshot,
    config: AppConfig,
}

impl OverlayApp {
    fn draw_all(&mut self, qh: &QueueHandle<Self>) -> Result<()> {
        let Some(model) = self.model.as_ref() else {
            return Ok(());
        };

        for (index, overlay) in self.overlays.iter_mut().enumerate() {
            let width = overlay.logical_size.width.max(1) as u32;
            let height = overlay.logical_size.height.max(1) as u32;
            let stride = width as i32 * 4;
            let (buffer, canvas) = overlay
                .pool
                .create_buffer(
                    width as i32,
                    height as i32,
                    stride,
                    wl_shm::Format::Argb8888,
                )
                .context("failed to create overlay buffer")?;
            let mut surface = ImageSurface::create(Format::ARgb32, width as i32, height as i32)?;
            let cr = CairoContext::new(&surface)?;

            let screenshot = if model.show_pointer {
                &mut overlay.with_pointer
            } else {
                &mut overlay.without_pointer
            };
            paint_background(&cr, screenshot, overlay.logical_size)?;
            paint_masks_and_border(&cr, overlay.logical_size, model.selection_on_output(index))?;

            let panel = if model.show_pointer {
                &mut self.panels.hide_pointer
            } else {
                &mut self.panels.show_pointer
            };
            let _panel_rect =
                paint_panel(&cr, panel, overlay.logical_size, model.dragging_selection())?;
            drop(cr);
            surface.flush();
            {
                let data = surface.data()?;
                canvas.copy_from_slice(&data);
            }

            overlay
                .layer
                .wl_surface()
                .damage_buffer(0, 0, width as i32, height as i32);
            overlay
                .layer
                .wl_surface()
                .frame(qh, overlay.layer.wl_surface().clone());
            buffer
                .attach_to(overlay.layer.wl_surface())
                .context("failed to attach overlay buffer")?;
            overlay.layer.commit();
        }

        Ok(())
    }

    fn handle_keysym(
        &mut self,
        keysym: smithay_client_toolkit::seat::keyboard::Keysym,
        repeat: bool,
    ) {
        let Some(model) = self.model.as_mut() else {
            return;
        };

        let keymap = &self.config.keymap;

        if matches_binding(&keymap.cancel, keysym, self.modifiers) {
            self.exit = Some(None);
            return;
        }
        if matches_binding(&keymap.toggle_pointer, keysym, self.modifiers) {
            model.toggle_pointer();
            return;
        }
        if matches_binding(&keymap.copy_only, keysym, self.modifiers) {
            self.exit = Some(Some(OverlayResult {
                region: model.capture_region(),
                show_pointer: model.show_pointer,
                write_to_disk: false,
            }));
            return;
        }
        if matches_binding(&keymap.confirm, keysym, self.modifiers)
            && !(repeat && matches!(model.button_state(), ButtonState::Down { .. }))
        {
            self.exit = Some(Some(OverlayResult {
                region: model.capture_region(),
                show_pointer: model.show_pointer,
                write_to_disk: true,
            }));
            return;
        }
        if matches_binding(&keymap.move_left, keysym, self.modifiers) {
            model.move_left();
            return;
        }
        if matches_binding(&keymap.move_right, keysym, self.modifiers) {
            model.move_right();
            return;
        }
        if matches_binding(&keymap.move_up, keysym, self.modifiers) {
            model.move_up();
            return;
        }
        if matches_binding(&keymap.move_down, keysym, self.modifiers) {
            model.move_down();
            return;
        }
        if matches_binding(&keymap.resize_left, keysym, self.modifiers) {
            model.resize_left();
            return;
        }
        if matches_binding(&keymap.resize_right, keysym, self.modifiers) {
            model.resize_right();
            return;
        }
        if matches_binding(&keymap.resize_up, keysym, self.modifiers) {
            model.resize_up();
            return;
        }
        if matches_binding(&keymap.resize_down, keysym, self.modifiers) {
            model.resize_down();
            return;
        }
        if matches_binding(&keymap.next_output, keysym, self.modifiers) {
            model.cycle_output(1);
            return;
        }
        if matches_binding(&keymap.previous_output, keysym, self.modifiers) {
            model.cycle_output(-1);
            return;
        }

        if keysym == smithay_client_toolkit::seat::keyboard::Keysym::space {
            model.set_move_mode(true);
        }
    }

    fn handle_surface_pointer_up(&mut self, overlay_index: usize, point: Point) {
        let Some(model) = self.model.as_mut() else {
            return;
        };
        let panel_rect = panel_rect_for_overlay(
            self.overlays[overlay_index].logical_size,
            if model.show_pointer {
                &self.panels.hide_pointer
            } else {
                &self.panels.show_pointer
            },
        );
        let over_button = capture_button_hit(panel_rect, point);
        match model.pointer_up(overlay_index, point, over_button) {
            PointerUpOutcome::None => {}
            PointerUpOutcome::Redraw => {}
            PointerUpOutcome::Confirm => {
                self.exit = Some(Some(OverlayResult {
                    region: model.capture_region(),
                    show_pointer: model.show_pointer,
                    write_to_disk: true,
                }));
            }
        }
    }
}

fn matches_binding(
    bindings: &[KeyBinding],
    keysym: smithay_client_toolkit::seat::keyboard::Keysym,
    modifiers: Modifiers,
) -> bool {
    bindings
        .iter()
        .any(|binding| binding.matches(keysym, modifiers))
}

fn logical_size(info: &OutputInfo) -> Option<Size> {
    info.logical_size.map(|(w, h)| Size::new(w, h))
}

fn point_from_position(position: (f64, f64)) -> Point {
    Point::new(position.0.round() as i32, position.1.round() as i32)
}

fn panel_rect_for_overlay(size: Size, panel: &PixelSurface) -> Rect {
    let point = crate::render::panel_location(size, Size::new(panel.width, panel.height));
    Rect::new(point.x, point.y, panel.width, panel.height)
}

impl CompositorHandler for OverlayApp {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for OverlayApp {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for OverlayApp {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.exit = Some(None);
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if let Some(surface) = self
            .overlays
            .iter_mut()
            .find(|surface| &surface.layer == layer)
        {
            surface.logical_size = Size::new(
                NonZeroU32::new(configure.new_size.0)
                    .map_or(surface.logical_size.width as u32, NonZeroU32::get)
                    as i32,
                NonZeroU32::new(configure.new_size.1)
                    .map_or(surface.logical_size.height as u32, NonZeroU32::get)
                    as i32,
            );
            surface.configured = true;
        }
    }
}

impl SeatHandler for OverlayApp {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat_state.get_keyboard(qh, &seat, None).ok();
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
        if capability == Capability::Touch && self.touch.is_none() {
            self.touch = self.seat_state.get_touch(qh, &seat).ok();
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            if let Some(keyboard) = self.keyboard.take() {
                keyboard.release();
            }
        }
        if capability == Capability::Pointer {
            if let Some(pointer) = self.pointer.take() {
                pointer.release();
            }
        }
        if capability == Capability::Touch {
            if let Some(touch) = self.touch.take() {
                touch.release();
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for OverlayApp {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[smithay_client_toolkit::seat::keyboard::Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.handle_keysym(event.keysym, false);
    }

    fn repeat_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.handle_keysym(event.keysym, true);
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        if event.keysym == smithay_client_toolkit::seat::keyboard::Keysym::space {
            if let Some(model) = self.model.as_mut() {
                model.set_move_mode(false);
            }
        }
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
        self.modifiers = modifiers;
    }
}

impl PointerHandler for OverlayApp {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let Some(index) = self
                .overlays
                .iter()
                .position(|overlay| event.surface == *overlay.layer.wl_surface())
            else {
                continue;
            };
            let point = point_from_position(event.position);

            match event.kind {
                PointerEventKind::Motion { .. } => {
                    if let Some(model) = self.model.as_mut() {
                        let _ = model.pointer_motion(index, point);
                    }
                }
                PointerEventKind::Press { button, .. } => {
                    if button == 0x110 {
                        if let Some(model) = self.model.as_mut() {
                            let panel_rect = panel_rect_for_overlay(
                                self.overlays[index].logical_size,
                                if model.show_pointer {
                                    &self.panels.hide_pointer
                                } else {
                                    &self.panels.show_pointer
                                },
                            );
                            let over_button = capture_button_hit(panel_rect, point);
                            let _ = model.pointer_down(index, point, over_button);
                        }
                    }
                }
                PointerEventKind::Release { button, .. } => {
                    if button == 0x110 {
                        self.handle_surface_pointer_up(index, point);
                    }
                }
                _ => {}
            }
        }
    }
}

impl TouchHandler for OverlayApp {
    fn down(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        surface: wl_surface::WlSurface,
        _id: i32,
        position: (f64, f64),
    ) {
        let Some(index) = self
            .overlays
            .iter()
            .position(|overlay| surface == *overlay.layer.wl_surface())
        else {
            return;
        };
        let point = point_from_position(position);
        if let Some(model) = self.model.as_mut() {
            let panel_rect = panel_rect_for_overlay(
                self.overlays[index].logical_size,
                if model.show_pointer {
                    &self.panels.hide_pointer
                } else {
                    &self.panels.show_pointer
                },
            );
            let over_button = capture_button_hit(panel_rect, point);
            let _ = model.pointer_down(index, point, over_button);
        }
    }

    fn up(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _serial: u32,
        _time: u32,
        _id: i32,
    ) {
    }

    fn motion(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _time: u32,
        _id: i32,
        position: (f64, f64),
    ) {
        if let Some(model) = self.model.as_mut() {
            let index = model.selected_output_index();
            let _ = model.pointer_motion(index, point_from_position(position));
        }
    }

    fn shape(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _id: i32,
        _major: f64,
        _minor: f64,
    ) {
    }

    fn orientation(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _touch: &wl_touch::WlTouch,
        _id: i32,
        _orientation: f64,
    ) {
    }

    fn cancel(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _touch: &wl_touch::WlTouch) {}
}

impl ShmHandler for OverlayApp {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(OverlayApp);
delegate_output!(OverlayApp);
delegate_shm!(OverlayApp);
delegate_seat!(OverlayApp);
delegate_keyboard!(OverlayApp);
delegate_pointer!(OverlayApp);
delegate_touch!(OverlayApp);
delegate_layer!(OverlayApp);
delegate_registry!(OverlayApp);

impl ProvidesRegistryState for OverlayApp {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

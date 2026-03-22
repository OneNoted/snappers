use crate::geometry::{Point, Rect};

const DEFAULT_CLICK_SIZE: i32 = 32;
const KEYBOARD_STEP: i32 = 16;

#[derive(Debug, Clone)]
pub struct OutputState {
    pub logical_rect: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Up,
    Down {
        output_index: usize,
        last_point: Point,
        on_capture_button: bool,
        moving: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerUpOutcome {
    None,
    Redraw,
    Confirm,
}

#[derive(Debug, Clone)]
pub struct SelectionModel {
    outputs: Vec<OutputState>,
    selected_output: usize,
    local_rect: Rect,
    drag_anchor: Point,
    button: ButtonState,
    pub show_pointer: bool,
}

impl SelectionModel {
    pub fn new(outputs: Vec<OutputState>, default_output: usize, show_pointer: bool) -> Self {
        let output = &outputs[default_output];
        let width = (output.logical_rect.width / 2).max(1);
        let height = (output.logical_rect.height / 2).max(1);
        let local_rect = Rect::new(
            output.logical_rect.width / 4,
            output.logical_rect.height / 4,
            width,
            height,
        );

        Self {
            outputs,
            selected_output: default_output,
            local_rect,
            drag_anchor: Point::new(local_rect.x, local_rect.y),
            button: ButtonState::Up,
            show_pointer,
        }
    }

    pub fn selected_output_index(&self) -> usize {
        self.selected_output
    }

    pub fn selection_on_output(&self, output_index: usize) -> Option<Rect> {
        (output_index == self.selected_output).then_some(self.local_rect)
    }

    pub fn dragging_selection(&self) -> bool {
        matches!(
            self.button,
            ButtonState::Down {
                on_capture_button: false,
                ..
            }
        )
    }

    pub fn pointer_down(
        &mut self,
        output_index: usize,
        point: Point,
        over_capture_button: bool,
    ) -> bool {
        if !matches!(self.button, ButtonState::Up) {
            return false;
        }

        self.selected_output = output_index;
        self.drag_anchor = point;
        self.local_rect = Rect::new(point.x, point.y, 1, 1);
        self.button = ButtonState::Down {
            output_index,
            last_point: point,
            on_capture_button: over_capture_button,
            moving: false,
        };
        !over_capture_button
    }

    pub fn pointer_motion(&mut self, output_index: usize, point: Point) -> bool {
        let ButtonState::Down {
            output_index: pressed_output,
            on_capture_button,
            moving,
            ..
        } = self.button
        else {
            return false;
        };

        if pressed_output != output_index {
            return false;
        }

        if on_capture_button {
            self.button = ButtonState::Down {
                output_index,
                last_point: point,
                on_capture_button,
                moving,
            };
            return false;
        }

        if moving {
            let delta_x = point.x - self.drag_anchor.x;
            let delta_y = point.y - self.drag_anchor.y;
            self.local_rect = Rect::new(
                self.local_rect.x + delta_x,
                self.local_rect.y + delta_y,
                self.local_rect.width,
                self.local_rect.height,
            )
            .clamp_within(self.local_bounds(self.selected_output));
            self.drag_anchor = point;
        } else {
            self.local_rect = Rect::from_corners(self.drag_anchor, point)
                .clamp_within(self.local_bounds(self.selected_output));
        }

        self.button = ButtonState::Down {
            output_index,
            last_point: point,
            on_capture_button,
            moving,
        };
        true
    }

    pub fn pointer_up(
        &mut self,
        output_index: usize,
        point: Point,
        still_over_capture_button: bool,
    ) -> PointerUpOutcome {
        let ButtonState::Down {
            output_index: pressed_output,
            on_capture_button,
            ..
        } = self.button
        else {
            return PointerUpOutcome::None;
        };

        if pressed_output != output_index {
            return PointerUpOutcome::None;
        }

        self.button = ButtonState::Up;
        if on_capture_button && still_over_capture_button {
            return PointerUpOutcome::Confirm;
        }

        if self.local_rect.width <= 1 && self.local_rect.height <= 1 {
            let bounds = self.local_bounds(self.selected_output);
            self.local_rect = Rect::new(
                point.x - DEFAULT_CLICK_SIZE / 2,
                point.y - DEFAULT_CLICK_SIZE / 2,
                DEFAULT_CLICK_SIZE,
                DEFAULT_CLICK_SIZE,
            )
            .clamp_within(bounds);
        }

        PointerUpOutcome::Redraw
    }

    pub fn set_move_mode(&mut self, moving: bool) {
        if let ButtonState::Down {
            output_index,
            last_point,
            on_capture_button,
            ..
        } = self.button
        {
            self.drag_anchor = last_point;
            self.button = ButtonState::Down {
                output_index,
                last_point,
                on_capture_button,
                moving,
            };
        }
    }

    pub fn toggle_pointer(&mut self) {
        self.show_pointer = !self.show_pointer;
    }

    pub fn move_left(&mut self) {
        self.nudge(-KEYBOARD_STEP, 0);
    }

    pub fn move_right(&mut self) {
        self.nudge(KEYBOARD_STEP, 0);
    }

    pub fn move_up(&mut self) {
        self.nudge(0, -KEYBOARD_STEP);
    }

    pub fn move_down(&mut self) {
        self.nudge(0, KEYBOARD_STEP);
    }

    pub fn resize_left(&mut self) {
        self.resize_by(-KEYBOARD_STEP, 0);
    }

    pub fn resize_right(&mut self) {
        self.resize_by(KEYBOARD_STEP, 0);
    }

    pub fn resize_up(&mut self) {
        self.resize_by(0, -KEYBOARD_STEP);
    }

    pub fn resize_down(&mut self) {
        self.resize_by(0, KEYBOARD_STEP);
    }

    pub fn cycle_output(&mut self, delta: isize) {
        let len = self.outputs.len() as isize;
        if len <= 1 {
            return;
        }

        let new_index = (self.selected_output as isize + delta).rem_euclid(len) as usize;
        if new_index == self.selected_output {
            return;
        }

        let current_bounds = self.local_bounds(self.selected_output);
        let target_bounds = self.local_bounds(new_index);
        let rel_x = self.local_rect.x as f64 / current_bounds.width as f64;
        let rel_y = self.local_rect.y as f64 / current_bounds.height as f64;

        let mut rect = Rect::new(
            (rel_x * target_bounds.width as f64).round() as i32,
            (rel_y * target_bounds.height as f64).round() as i32,
            self.local_rect.width.min(target_bounds.width),
            self.local_rect.height.min(target_bounds.height),
        );
        rect = rect.clamp_within(target_bounds);
        self.selected_output = new_index;
        self.local_rect = rect;
    }

    pub fn capture_region(&self) -> Rect {
        let bounds = self.output_global_bounds(self.selected_output);
        Rect::new(
            bounds.x + self.local_rect.x,
            bounds.y + self.local_rect.y,
            self.local_rect.width,
            self.local_rect.height,
        )
    }

    pub fn button_state(&self) -> ButtonState {
        self.button
    }

    fn nudge(&mut self, dx: i32, dy: i32) {
        self.local_rect = Rect::new(
            self.local_rect.x + dx,
            self.local_rect.y + dy,
            self.local_rect.width,
            self.local_rect.height,
        )
        .clamp_within(self.local_bounds(self.selected_output));
    }

    fn resize_by(&mut self, dw: i32, dh: i32) {
        let bounds = self.local_bounds(self.selected_output);
        self.local_rect = Rect::new(
            self.local_rect.x,
            self.local_rect.y,
            (self.local_rect.width + dw).max(1),
            (self.local_rect.height + dh).max(1),
        )
        .clamp_within(bounds);
    }

    fn output_global_bounds(&self, output_index: usize) -> Rect {
        self.outputs[output_index].logical_rect
    }

    fn local_bounds(&self, output_index: usize) -> Rect {
        let output = self.output_global_bounds(output_index);
        Rect::new(0, 0, output.width, output.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outputs() -> Vec<OutputState> {
        vec![
            OutputState {
                logical_rect: Rect::new(0, 0, 1920, 1080),
            },
            OutputState {
                logical_rect: Rect::new(1920, 0, 2560, 1440),
            },
        ]
    }

    #[test]
    fn default_rect_is_centered() {
        let model = SelectionModel::new(outputs(), 0, true);
        assert_eq!(model.capture_region(), Rect::new(480, 270, 960, 540));
    }

    #[test]
    fn click_expands_to_default_size() {
        let mut model = SelectionModel::new(outputs(), 0, true);
        assert!(model.pointer_down(0, Point::new(50, 50), false));
        assert_eq!(
            model.pointer_up(0, Point::new(50, 50), false),
            PointerUpOutcome::Redraw
        );
        assert_eq!(model.capture_region(), Rect::new(34, 34, 32, 32));
    }

    #[test]
    fn cycles_outputs_preserving_relative_origin() {
        let mut model = SelectionModel::new(outputs(), 0, true);
        model.cycle_output(1);
        assert_eq!(model.selected_output_index(), 1);
        assert_eq!(model.capture_region(), Rect::new(2560, 360, 960, 540));
    }

    #[test]
    fn resize_clamps_to_output() {
        let mut model = SelectionModel::new(outputs(), 0, true);
        for _ in 0..100 {
            model.resize_right();
        }
        assert!(model.capture_region().width <= 1920);
    }

    #[test]
    fn capture_region_includes_global_output_origin() {
        let outputs = vec![OutputState {
            logical_rect: Rect::new(-1600, 200, 1600, 900),
        }];
        let model = SelectionModel::new(outputs, 0, true);
        assert_eq!(model.capture_region(), Rect::new(-1200, 425, 800, 450));
    }
}

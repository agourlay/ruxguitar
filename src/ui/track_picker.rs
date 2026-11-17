//! A track dropdown with a wider, right-aligned menu and per-track solo/mute
//! actions.

use crate::ui::selections::TrackSelection;
use iced::advanced::text::Shaping;
use iced::advanced::{
    Clipboard, Layout, Overlay, Renderer as GfxRenderer, Shell, Widget, layout, overlay, renderer,
    text,
    text::{Renderer as TextRenderer, paragraph},
    widget::tree,
};
use iced::alignment;
use iced::widget::overlay::menu::Catalog as MenuCatalog;
use iced::widget::pick_list::{Catalog as PickListCatalog, Status};
use iced::{
    Background, Border, Element, Event, Length, Padding, Pixels, Point, Rectangle, Size, Theme,
    Vector, keyboard, mouse, touch, window,
};

/// Open menu width (wider than the trigger when there is room).
const MENU_MAX_WIDTH: f32 = 420.0;
const ROW_HEIGHT: f32 = 34.0;
const ACTION_WIDTH: f32 = 30.0;
const ROW_PADDING: f32 = 8.0;
const ACTION_GAP: f32 = 4.0;
const TEXT_SIZE: f32 = 14.0;

pub struct TrackPicker<'a, Message> {
    tracks: &'a [TrackSelection],
    selected: &'a TrackSelection,
    is_solo: Box<dyn Fn(usize) -> bool + 'a>,
    is_muted: Box<dyn Fn(usize) -> bool + 'a>,
    on_select: Box<dyn Fn(TrackSelection) -> Message + 'a>,
    on_solo: Box<dyn Fn(usize) -> Message + 'a>,
    on_mute: Box<dyn Fn(usize) -> Message + 'a>,
    last_status: Option<Status>,
}

impl<'a, Message> TrackPicker<'a, Message>
where
    Message: Clone + 'a,
{
    #[must_use]
    pub fn new(
        tracks: &'a [TrackSelection],
        selected: &'a TrackSelection,
        is_solo: impl Fn(usize) -> bool + 'a,
        is_muted: impl Fn(usize) -> bool + 'a,
        on_select: impl Fn(TrackSelection) -> Message + 'a,
        on_solo: impl Fn(usize) -> Message + 'a,
        on_mute: impl Fn(usize) -> Message + 'a,
    ) -> Self {
        Self {
            tracks,
            selected,
            is_solo: Box::new(is_solo),
            is_muted: Box::new(is_muted),
            on_select: Box::new(on_select),
            on_solo: Box::new(on_solo),
            on_mute: Box::new(on_mute),
            last_status: None,
        }
    }

    pub fn view(self) -> Element<'a, Message> {
        Element::new(self)
    }
}

type LabelParagraph = paragraph::Plain<<iced::Renderer as TextRenderer>::Paragraph>;

#[derive(Debug)]
struct State {
    is_open: bool,
    hovered_index: Option<usize>,
    /// Vertical scroll of the open menu, for songs with more tracks than fit.
    scroll_offset: f32,
    keyboard_modifiers: keyboard::Modifiers,
    labels: Vec<LabelParagraph>,
}

impl State {
    fn new() -> Self {
        Self {
            is_open: false,
            hovered_index: None,
            scroll_offset: 0.0,
            keyboard_modifiers: keyboard::Modifiers::default(),
            labels: Vec::new(),
        }
    }
}

fn wheel_delta_pixels(delta: mouse::ScrollDelta) -> f32 {
    match delta {
        mouse::ScrollDelta::Lines { y, .. } => y * ROW_HEIGHT,
        mouse::ScrollDelta::Pixels { y, .. } => y,
    }
}

impl<'a, Message> Widget<Message, Theme, iced::Renderer> for TrackPicker<'a, Message>
where
    Message: Clone + 'a,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::new())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Shrink,
            height: Length::Shrink,
        }
    }

    fn layout(
        &mut self,
        tree: &mut tree::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State>();
        let text_size = Pixels(TEXT_SIZE);
        let padding = Padding::from([5, 10]);
        let line_height = text::LineHeight::default();
        let font = renderer.default_font();

        state
            .labels
            .resize_with(self.tracks.len(), Default::default);

        let option_text = text::Text {
            content: "",
            bounds: Size::new(f32::INFINITY, line_height.to_absolute(text_size).into()),
            size: text_size,
            line_height,
            font,
            align_x: text::Alignment::Default,
            align_y: alignment::Vertical::Center,
            shaping: Shaping::default(),
            wrapping: text::Wrapping::None,
        };

        for (track, paragraph) in self.tracks.iter().zip(state.labels.iter_mut()) {
            let label = track.to_string();
            let _ = paragraph.update(text::Text {
                content: &label,
                ..option_text
            });
        }

        let labels_width = state.labels.iter().fold(0.0_f32, |width, paragraph| {
            f32::max(width, paragraph.min_width())
        });

        let intrinsic = Size::new(
            labels_width + text_size.0 + padding.left,
            f32::from(line_height.to_absolute(text_size)),
        );

        let size = limits
            .width(Length::Shrink)
            .shrink(padding)
            .resolve(Length::Shrink, Length::Shrink, intrinsic)
            .expand(padding);

        layout::Node::new(size)
    }

    fn update(
        &mut self,
        tree: &mut tree::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                if state.is_open {
                    state.is_open = false;
                    shell.capture_event();
                } else if cursor.is_over(layout.bounds()) {
                    state.is_open = true;
                    state.hovered_index = self
                        .tracks
                        .iter()
                        .position(|t| t.index == self.selected.index);
                    // start with the selected track at the top, clamped by the menu layout
                    state.scroll_offset = state.hovered_index.unwrap_or(0) as f32 * ROW_HEIGHT;
                    shell.capture_event();
                }
            }
            // like the stock pick_list: Ctrl/Cmd + wheel cycles through the tracks
            Event::Mouse(mouse::Event::WheelScrolled { delta })
                if state.keyboard_modifiers.command()
                    && !state.is_open
                    && cursor.is_over(layout.bounds()) =>
            {
                let position = self
                    .tracks
                    .iter()
                    .position(|t| t.index == self.selected.index);
                let next = if wheel_delta_pixels(*delta) < 0.0 {
                    position.map_or(Some(0), |p| Some(p + 1))
                } else {
                    position.and_then(|p| p.checked_sub(1))
                };
                if let Some(track) = next.and_then(|i| self.tracks.get(i)) {
                    shell.publish((self.on_select)(track.clone()));
                }
                shell.capture_event();
            }
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.keyboard_modifiers = *modifiers;
            }
            _ => {}
        }

        let status = if state.is_open {
            Status::Opened {
                is_hovered: cursor.is_over(layout.bounds()),
            }
        } else if cursor.is_over(layout.bounds()) {
            Status::Hovered
        } else {
            Status::Active
        };

        if let Event::Window(window::Event::RedrawRequested(_)) = event {
            self.last_status = Some(status);
        } else if self.last_status.is_some_and(|last| last != status) {
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        _tree: &tree::Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        use text::Renderer as _;

        let bounds = layout.bounds();
        let status = self.last_status.unwrap_or(Status::Active);
        let style = PickListCatalog::style(theme, &<Theme as PickListCatalog>::default(), status);
        renderer.fill_quad(
            renderer::Quad {
                bounds,
                border: style.border,
                ..renderer::Quad::default()
            },
            style.background,
        );
        let text_size = Pixels(TEXT_SIZE);
        let text_bounds = Size::new((bounds.width - text_size.0 - 20.0).max(0.0), text_size.0);
        renderer.fill_text(
            text::Text {
                content: self.selected.to_string(),
                size: text_size,
                line_height: text::LineHeight::default(),
                font: renderer.default_font(),
                bounds: text_bounds,
                align_x: text::Alignment::Default,
                align_y: alignment::Vertical::Center,
                shaping: Shaping::default(),
                wrapping: text::Wrapping::None,
            },
            Point::new(bounds.x + 10.0, bounds.center_y()),
            style.text_color,
            *viewport,
        );
        renderer.fill_text(
            text::Text {
                content: "\u{25BE}".to_string(),
                size: text_size,
                line_height: text::LineHeight::default(),
                font: renderer.default_font(),
                bounds: Size::new(bounds.width, text_size.0),
                align_x: text::Alignment::Right,
                align_y: alignment::Vertical::Center,
                shaping: Shaping::Basic,
                wrapping: text::Wrapping::default(),
            },
            Point::new(bounds.x + bounds.width - 10.0, bounds.center_y()),
            style.handle_color,
            *viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut tree::Tree,
        layout: Layout<'_>,
        _renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, iced::Renderer>> {
        let state = tree.state.downcast_mut::<State>();
        if !state.is_open {
            return None;
        }
        let bounds = layout.bounds();
        let menu_width = MENU_MAX_WIDTH.max(bounds.width).min(viewport.width - 8.0);
        let menu = TrackMenu {
            width: menu_width,
            tracks: self.tracks,
            selected: self.selected,
            hovered_index: &mut state.hovered_index,
            scroll_offset: &mut state.scroll_offset,
            is_solo: &self.is_solo,
            is_muted: &self.is_muted,
            on_select: &self.on_select,
            on_solo: &self.on_solo,
            on_mute: &self.on_mute,
            is_open: &mut state.is_open,
        };
        Some(overlay::Element::new(Box::new(TrackMenuOverlay {
            position: bounds.position() + translation,
            viewport: *viewport,
            target_width: bounds.width,
            target_height: bounds.height,
            menu_width,
            tree: tree::Tree::empty(),
            list: menu,
        })))
    }
}

struct TrackMenu<'a, Message> {
    width: f32,
    tracks: &'a [TrackSelection],
    selected: &'a TrackSelection,
    hovered_index: &'a mut Option<usize>,
    scroll_offset: &'a mut f32,
    is_solo: &'a dyn Fn(usize) -> bool,
    is_muted: &'a dyn Fn(usize) -> bool,
    on_select: &'a dyn Fn(TrackSelection) -> Message,
    on_solo: &'a dyn Fn(usize) -> Message,
    on_mute: &'a dyn Fn(usize) -> Message,
    is_open: &'a mut bool,
}

#[derive(Debug, Clone, Copy)]
enum RowHit {
    Select(usize),
    Solo(usize),
    Mute(usize),
}

impl RowHit {
    fn index(self) -> usize {
        match self {
            Self::Select(i) | Self::Solo(i) | Self::Mute(i) => i,
        }
    }
}

impl<Message> TrackMenu<'_, Message> {
    fn content_height(&self) -> f32 {
        ROW_HEIGHT * self.tracks.len() as f32
    }

    fn row_index_at(&self, y_in_bounds: f32) -> usize {
        ((y_in_bounds + *self.scroll_offset) / ROW_HEIGHT) as usize
    }

    fn row_hit(&self, cursor: Point, bounds: Rectangle) -> Option<RowHit> {
        if !bounds.contains(cursor) {
            return None;
        }
        let index = self.row_index_at(cursor.y - bounds.y);
        let local_x = cursor.x - bounds.x;
        let mute_start = bounds.width - ROW_PADDING - ACTION_WIDTH;
        let solo_start = mute_start - ACTION_GAP - ACTION_WIDTH;
        if local_x >= mute_start {
            Some(RowHit::Mute(index))
        } else if local_x >= solo_start {
            Some(RowHit::Solo(index))
        } else {
            Some(RowHit::Select(index))
        }
    }
}

impl<Message> TrackMenu<'_, Message> {
    /// Hover the row under the cursor; returns whether it changed.
    fn update_hover(&mut self, cursor: mouse::Cursor, bounds: Rectangle) -> bool {
        if let Some(cursor_position) = cursor.position_in(bounds) {
            let index = self.row_index_at(cursor_position.y);
            if *self.hovered_index != Some(index) && index < self.tracks.len() {
                *self.hovered_index = Some(index);
                return true;
            }
        }
        false
    }
}

impl<Message> Widget<Message, Theme, iced::Renderer> for TrackMenu<'_, Message>
where
    Message: Clone,
{
    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fixed(self.width),
            height: Length::Shrink,
        }
    }

    fn layout(
        &mut self,
        _tree: &mut tree::Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let height = ROW_HEIGHT * self.tracks.len().max(1) as f32;
        let size = limits
            .width(Length::Fixed(self.width))
            .height(Length::Shrink)
            .resolve(
                Length::Fixed(self.width),
                Length::Shrink,
                Size::new(self.width, height),
            )
            .expand(Padding::ZERO);
        // the limits may cut the menu short: keep the scroll within the hidden rows
        *self.scroll_offset = self
            .scroll_offset
            .clamp(0.0, (self.content_height() - size.height).max(0.0));
        layout::Node::new(size)
    }

    fn update(
        &mut self,
        _tree: &mut tree::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
            | Event::Touch(touch::Event::FingerPressed { .. }) => {
                if let Some(cursor_position) = cursor.position()
                    && let Some(hit) = self.row_hit(cursor_position, bounds)
                    && self.tracks.get(hit.index()).is_some()
                {
                    match hit {
                        RowHit::Select(index) => {
                            shell.publish((self.on_select)(self.tracks[index].clone()));
                            *self.is_open = false;
                        }
                        RowHit::Solo(index) => {
                            shell.publish((self.on_solo)(self.tracks[index].index));
                        }
                        RowHit::Mute(index) => {
                            shell.publish((self.on_mute)(self.tracks[index].index));
                        }
                    }
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) if cursor.is_over(bounds) => {
                let max_offset = (self.content_height() - bounds.height).max(0.0);
                let offset =
                    (*self.scroll_offset - wheel_delta_pixels(*delta)).clamp(0.0, max_offset);
                if offset != *self.scroll_offset {
                    *self.scroll_offset = offset;
                    self.update_hover(cursor, bounds);
                    shell.request_redraw();
                }
                shell.capture_event();
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if self.update_hover(cursor, bounds) => {
                shell.request_redraw();
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        _tree: &tree::Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        use text::Renderer as _;

        let style = MenuCatalog::style(theme, &<Theme as MenuCatalog>::default());
        let bounds = layout.bounds();
        let text_size = Pixels(TEXT_SIZE);
        let font = renderer.default_font();
        let palette = theme.extended_palette();
        let mute_start = bounds.width - ROW_PADDING - ACTION_WIDTH;
        let solo_start = mute_start - ACTION_GAP - ACTION_WIDTH;
        let label_width = solo_start - ROW_PADDING;
        let first_visible = self.row_index_at(0.0);
        let visible_rows = (bounds.height / ROW_HEIGHT).ceil() as usize + 1;

        // clip the rows scrolled out of the menu
        renderer.with_layer(bounds, |renderer| {
            for (index, track) in self
                .tracks
                .iter()
                .enumerate()
                .skip(first_visible)
                .take(visible_rows)
            {
                let row_bounds = Rectangle {
                    x: bounds.x,
                    y: bounds.y + ROW_HEIGHT * index as f32 - *self.scroll_offset,
                    width: bounds.width,
                    height: ROW_HEIGHT,
                };

                let is_hovered = *self.hovered_index == Some(index);
                let is_current = track.index == self.selected.index;
                if is_hovered || is_current {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle {
                                x: row_bounds.x + style.border.width,
                                width: row_bounds.width - style.border.width * 2.0,
                                ..row_bounds
                            },
                            ..renderer::Quad::default()
                        },
                        if is_current {
                            style.selected_background
                        } else {
                            palette.primary.weak.color.into()
                        },
                    );
                }

                // clip long track names before the solo/mute buttons
                let label_bounds = Rectangle {
                    x: row_bounds.x + ROW_PADDING,
                    width: label_width,
                    ..row_bounds
                };
                renderer.fill_text(
                    text::Text {
                        content: track.to_string(),
                        size: text_size,
                        line_height: text::LineHeight::default(),
                        font,
                        bounds: label_bounds.size(),
                        align_x: text::Alignment::Default,
                        align_y: alignment::Vertical::Center,
                        shaping: Shaping::default(),
                        wrapping: text::Wrapping::None,
                    },
                    Point::new(label_bounds.x, label_bounds.center_y()),
                    if is_current {
                        style.selected_text_color
                    } else {
                        style.text_color
                    },
                    label_bounds.intersection(viewport).unwrap_or_default(),
                );

                let action_y = row_bounds.y + (ROW_HEIGHT - ACTION_WIDTH) / 2.0;
                draw_action_button(
                    renderer,
                    theme,
                    viewport,
                    Rectangle {
                        x: row_bounds.x + solo_start,
                        y: action_y,
                        width: ACTION_WIDTH,
                        height: ACTION_WIDTH,
                    },
                    "S",
                    (self.is_solo)(track.index),
                );
                draw_action_button(
                    renderer,
                    theme,
                    viewport,
                    Rectangle {
                        x: row_bounds.x + mute_start,
                        y: action_y,
                        width: ACTION_WIDTH,
                        height: ACTION_WIDTH,
                    },
                    "M",
                    (self.is_muted)(track.index),
                );
            }
        });
    }
}

fn draw_action_button(
    renderer: &mut iced::Renderer,
    theme: &Theme,
    viewport: &Rectangle,
    bounds: Rectangle,
    label: &str,
    active: bool,
) {
    use text::Renderer as _;

    let palette = theme.extended_palette();
    let (background, text_color) = if active {
        (
            Background::from(palette.primary.strong.color),
            palette.primary.strong.text,
        )
    } else {
        (
            Background::from(palette.background.weak.color),
            palette.background.weak.text,
        )
    };
    renderer.fill_quad(
        renderer::Quad {
            bounds,
            border: Border {
                radius: 2.0.into(),
                width: 1.0,
                color: palette.background.strong.color,
            },
            ..renderer::Quad::default()
        },
        background,
    );
    renderer.fill_text(
        text::Text {
            content: label.to_string(),
            size: Pixels(12.0),
            line_height: text::LineHeight::default(),
            font: renderer.default_font(),
            bounds: bounds.size(),
            align_x: text::Alignment::Center,
            align_y: alignment::Vertical::Center,
            shaping: Shaping::Basic,
            wrapping: text::Wrapping::default(),
        },
        Point::new(bounds.center_x(), bounds.center_y()),
        text_color,
        *viewport,
    );
}

struct TrackMenuOverlay<'a, Message> {
    position: Point,
    viewport: Rectangle,
    target_width: f32,
    target_height: f32,
    menu_width: f32,
    tree: tree::Tree,
    list: TrackMenu<'a, Message>,
}

impl<Message> Overlay<Message, Theme, iced::Renderer> for TrackMenuOverlay<'_, Message>
where
    Message: Clone,
{
    fn layout(&mut self, renderer: &iced::Renderer, bounds: Size) -> layout::Node {
        let space_below = bounds.height - (self.position.y + self.target_height);
        let space_above = self.position.y;
        let limits = layout::Limits::new(
            Size::ZERO,
            Size::new(
                bounds.width,
                if space_below > space_above {
                    space_below
                } else {
                    space_above
                },
            ),
        )
        .width(self.menu_width);

        let node = self.list.layout(&mut self.tree, renderer, &limits);
        let size = node.size();
        let mut x = self.position.x + self.target_width - size.width;
        x = x.clamp(0.0, (bounds.width - size.width).max(0.0));
        let y = if space_below > space_above {
            self.position.y + self.target_height
        } else {
            self.position.y - size.height
        };
        node.move_to(Point::new(x, y.max(0.0)))
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        self.list.update(
            &mut self.tree,
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            &self.viewport,
        );
    }

    fn draw(
        &self,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        defaults: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let style = MenuCatalog::style(theme, &<Theme as MenuCatalog>::default());
        renderer.fill_quad(
            renderer::Quad {
                bounds: layout.bounds(),
                border: style.border,
                shadow: style.shadow,
                ..renderer::Quad::default()
            },
            style.background,
        );
        self.list.draw(
            &self.tree,
            renderer,
            theme,
            defaults,
            layout,
            cursor,
            &self.viewport,
        );
    }
}

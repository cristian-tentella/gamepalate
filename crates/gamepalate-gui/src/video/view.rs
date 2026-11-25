use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::advanced::image::{Image, Renderer as _};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, mouse};
use iced::widget::shader::{self, Pipeline, Primitive, Viewport};
use iced::widget::{
    Shader, button, column, container, mouse_area, row, slider, stack, text, tooltip,
};
use iced::window::{self, RedrawRequest};
use iced::{
    Center, Color, Element, Event, Fill, Length, Point, Rectangle, Size, Theme, padding, wgpu,
};

use super::{Frame, IDLE, Message, Options, Player, Source, State};
use crate::{font, style};

const SOFTWARE_MAX_HEIGHT: u32 = 360;
const BAR_INSET: f32 = 12.0;
const BAR_SPACING: f32 = 8.0;
const POSITION_STEP: f64 = 0.1;
const BAR_ICON: f32 = 12.0;
const PLAY: char = '\u{E037}';
const PAUSE: char = '\u{E034}';
const VOLUME_UP: char = '\u{E050}';
const VOLUME_OFF: char = '\u{E04F}';
const REPLAY: char = '\u{E042}';
const VOLUME_WIDTH: f32 = 72.0;
const VOLUME_STEP: f32 = 0.01;
const SPINNER_DELAY: Duration = Duration::from_millis(300);
const SPINNER_SIZE: f32 = 40.0;
const SPINNER_STROKE: f32 = 4.0;
const SPINNER_TURN: f32 = 1.568;
const SPINNER_CYCLE: f32 = 1.333;
const SPINNER_MAX_SWEEP: f32 = 270.0;
const SPINNER_MIN_SWEEP: f32 = 10.0;
const LOOP_PLAY: f32 = 48.0;
const LOOP_PLAY_SHADOW: f32 = 2.0;
const LOOP_INSET: f32 = 8.0;
/// The textures of a player not drawn for this long are let go.
const TEXTURES_KEPT: Duration = Duration::from_secs(5);

const SHADER: &str = r#"
@group(0) @binding(0) var plane_y: texture_2d<f32>;
@group(0) @binding(1) var plane_u: texture_2d<f32>;
@group(0) @binding(2) var plane_v: texture_2d<f32>;
@group(0) @binding(3) var planes: sampler;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex fn vertex(@builtin(vertex_index) index: u32) -> Vertex {
    var corners = array<vec2<f32>, 6>(
        vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0),
        vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
    );
    let uv = corners[index];
    var out: Vertex;
    out.uv = uv;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    return out;
}

// BT.709, limited range.
@fragment fn fragment(in: Vertex) -> @location(0) vec4<f32> {
    let y = (textureSample(plane_y, planes, in.uv).r - 0.0625) * 1.1644;
    let u = textureSample(plane_u, planes, in.uv).r - 0.5;
    let v = textureSample(plane_v, planes, in.uv).r - 0.5;
    var rgb = clamp(
        vec3<f32>(y + 1.7927 * v, y - 0.2132 * u - 0.5329 * v, y + 2.1124 * u),
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
    if (SRGB) {
        rgb = pow(rgb, vec3<f32>(2.2));
    }
    return vec4<f32>(rgb, 1.0);
}
"#;

/// What the trailer is drawn on: its height in physical pixels and whether the renderer is tiny-skia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Surface {
    pub height: u32,
    pub software: bool,
}

impl Surface {
    pub fn source(&self, manifest_url: &str) -> Source {
        let height = match self.software {
            true => self.height.min(SOFTWARE_MAX_HEIGHT),
            false => self.height,
        };

        Source::Dash {
            manifest_url: manifest_url.to_string(),
            height,
        }
    }

    pub fn options(&self, muted: bool, volume: f32) -> Options {
        Options {
            muted,
            volume,
            rgba: self.software,
        }
    }
}

/// A transparent layer that reports the [`Surface`] under it, so that the rendition is chosen at Play.
pub fn probe<'a, M: 'a>(scale: f32, on_measure: impl Fn(Surface) -> M + 'a) -> Element<'a, M> {
    Element::new(Probe {
        scale,
        on_measure: Box::new(on_measure),
    })
}

struct Probe<'a, M> {
    scale: f32,
    on_measure: Box<dyn Fn(Surface) -> M + 'a>,
}

impl<M> Widget<M, Theme, iced::Renderer> for Probe<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Option<Surface>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(None::<Surface>)
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, Fill, Fill)
    }

    fn draw(
        &self,
        _tree: &Tree,
        _renderer: &mut iced::Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _viewport: &Rectangle,
    ) {
        if !matches!(event, Event::Window(window::Event::RedrawRequested(_))) {
            return;
        }
        let surface = Surface {
            height: physical(layout.bounds().height, self.scale),
            software: is_software(renderer),
        };
        let last = tree.state.downcast_mut::<Option<Surface>>();
        if *last != Some(surface) {
            *last = Some(surface);
            shell.publish((self.on_measure)(surface));
        }
    }
}

fn physical(height: f32, scale: f32) -> u32 {
    (height * scale).round().max(0.0) as u32
}

fn is_software(renderer: &iced::Renderer) -> bool {
    matches!(renderer, iced::Renderer::Secondary(_))
}

/// Whether the frames ask for a redraw at the next frame's time rather than at every vsync:
/// tiny-skia has no vsync, and a loop without sound is often a slow GIF (10–25 fps).
fn paced(software: bool, source: &Source) -> bool {
    software || !source.has_sound()
}

/// The redraw the frames ask for after a redraw: none while nothing moves, the next vsync,
/// or the next frame's time when [`paced`].
fn redraw(
    paced: bool,
    state: &State,
    old_frame: bool,
    next_frame_at: Option<Instant>,
    now: Instant,
) -> Option<RedrawRequest> {
    let moving = matches!(state, State::Loading | State::Playing) || old_frame;
    match (moving, paced) {
        (false, _) => None,
        (true, false) => Some(RedrawRequest::NextFrame),
        (true, true) => Some(RedrawRequest::At(next_frame_at.unwrap_or(now + IDLE))),
    }
}

/// The largest rectangle with the frame's shape, centred in the bounds.
fn fit(bounds: Rectangle, width: u32, height: u32) -> Rectangle {
    if width == 0 || height == 0 {
        return bounds;
    }
    let scale = (bounds.width / width as f32).min(bounds.height / height as f32);
    let size = Size::new(width as f32 * scale, height as f32 * scale);

    Rectangle::new(
        Point::new(
            bounds.x + (bounds.width - size.width) / 2.0,
            bounds.y + (bounds.height - size.height) / 2.0,
        ),
        size,
    )
}

pub fn clock(position: Duration, duration: Duration) -> String {
    format!("{} / {}", minutes(position), minutes(duration))
}

fn minutes(time: Duration) -> String {
    let seconds = time.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

impl Player {
    /// The player fills its space; `poster` shows while the trailer loads.
    pub fn view<'a>(&'a self, poster: Element<'a, Message>) -> Element<'a, Message> {
        self.views
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let state = self.state();
        let content: Element<'a, Message> = match &state {
            State::Failed(_) => failed("Couldn't play the trailer"),
            State::Loading => stack![
                poster,
                self.frames(),
                container(spinner(self.loading_since)).center(Fill),
            ]
            .into(),
            State::Ended => stack![self.frames(), container(replay()).center(Fill)].into(),
            State::Playing | State::Paused => {
                let screen = mouse_area(self.frames()).on_press(Message::Toggle);
                let bar = (self.hovered || state == State::Paused).then(|| self.bar(&state));
                stack![screen].push(bar).into()
            }
        };

        mouse_area(container(content).width(Fill).height(Fill).clip(true))
            .on_enter(Message::Hovered(true))
            .on_exit(Message::Hovered(false))
            .into()
    }

    /// A video of the description: no bar, the play button while it stands still and, under the
    /// mouse, a small pause button in the corner. `playing` is whether it should play.
    pub fn view_loop<'a>(
        &'a self,
        poster: Element<'a, Message>,
        playing: bool,
    ) -> Element<'a, Message> {
        self.views
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let state = self.state();
        let content: Element<'a, Message> = match (&state, playing) {
            (State::Failed(_), _) => failed("Couldn't play the video"),
            (_, false) => stack![poster, self.frames(), play_button()].into(),
            (State::Loading, true) => stack![
                poster,
                self.frames(),
                container(spinner(self.loading_since)).center(Fill),
            ]
            .into(),
            (_, true) => stack![poster, self.frames()]
                .push(self.hovered.then(pause_corner))
                .into(),
        };

        mouse_area(container(content).width(Fill).height(Fill).clip(true))
            .on_enter(Message::Hovered(true))
            .on_exit(Message::Hovered(false))
            .into()
    }

    fn frames(&self) -> Element<'_, Message> {
        Element::new(Frames { player: self })
    }

    fn bar(&self, state: &State) -> Element<'_, Message> {
        let duration = self.duration().unwrap_or_default().as_secs_f64();
        let position = self
            .scrub
            .unwrap_or_else(|| self.position().as_secs_f64())
            .min(duration);
        let toggle = match state {
            State::Paused => (PLAY, "Play"),
            _ => (PAUSE, "Pause"),
        };
        let mute = match self.is_silent() {
            true => (VOLUME_OFF, "Unmute"),
            false => (VOLUME_UP, "Mute"),
        };
        let level = match self.is_muted() {
            true => 0.0,
            false => self.volume(),
        };
        let sound = row![
            bar_button(mute, Message::Mute),
            slider(0.0..=1.0, level, Message::Volume)
                .step(VOLUME_STEP)
                .width(VOLUME_WIDTH)
                .style(style::video_position),
        ]
        .spacing(4)
        .align_y(Center);

        container(
            container(
                row![
                    bar_button(toggle, Message::Toggle),
                    slider(0.0..=duration, position, Message::Scrub)
                        .on_release(Message::Seek)
                        .step(POSITION_STEP)
                        .style(style::video_position),
                    text(clock(
                        Duration::from_secs_f64(position.max(0.0)),
                        Duration::from_secs_f64(duration)
                    ))
                    .size(font::NOTE_SIZE),
                    sound,
                ]
                .spacing(BAR_SPACING)
                .align_y(Center),
            )
            .padding([4, 8])
            .style(style::video_bar),
        )
        .padding(BAR_INSET)
        .align_bottom(Fill)
        .width(Fill)
        .into()
    }
}

fn bar_button<'a>((icon, tip): (char, &'a str), message: Message) -> Element<'a, Message> {
    tooltip(
        button(text(icon).font(font::ICONS).size(font::icon(BAR_ICON)))
            .padding([4, 6])
            .style(style::text_button)
            .on_press(message),
        container(text(tip).size(font::NOTE_SIZE))
            .padding([6, 10])
            .style(style::popup),
        tooltip::Position::Top,
    )
    .into()
}

fn replay<'a>() -> Element<'a, Message> {
    button(
        row![
            text(REPLAY).font(font::ICONS).size(font::icon(BAR_ICON)),
            text("Replay").size(font::NOTE_SIZE),
        ]
        .spacing(6)
        .align_y(Center),
    )
    .padding([8, 16])
    .style(style::arrow)
    .on_press(Message::Replay)
    .into()
}

/// The poster of a video of the description that has no player yet, with the play button.
pub fn still(poster: Element<'_, Message>) -> Element<'_, Message> {
    stack![poster, play_button()].into()
}

/// The whole video is the button, so that a click anywhere plays it.
fn play_button<'a>() -> Element<'a, Message> {
    let icon = |color: Color| text(PLAY).font(font::ICONS).size(LOOP_PLAY).color(color);
    let play = tooltip(
        stack![
            container(icon(Color::BLACK.scale_alpha(0.4))).padding(padding::top(LOOP_PLAY_SHADOW)),
            icon(Color::WHITE),
        ],
        container(text("Play").size(font::NOTE_SIZE))
            .padding([6, 10])
            .style(style::popup),
        tooltip::Position::Bottom,
    );

    mouse_area(container(play).center(Fill))
        .on_press(Message::Toggle)
        .interaction(mouse::Interaction::Pointer)
        .into()
}

fn pause_corner<'a>() -> Element<'a, Message> {
    container(container(bar_button((PAUSE, "Pause"), Message::Toggle)).style(style::video_bar))
        .padding(LOOP_INSET)
        .align_right(Fill)
        .align_bottom(Fill)
        .into()
}

fn failed<'a>(label: &'a str) -> Element<'a, Message> {
    container(
        column![
            text(label).size(font::NOTE_SIZE).style(style::muted),
            button(text("Retry").size(font::NOTE_SIZE))
                .style(style::primary)
                .padding([4, 12])
                .on_press(Message::Retry),
        ]
        .spacing(8)
        .align_x(Center),
    )
    .center(Fill)
    .style(style::picture)
    .into()
}

/// The circular indicator of Material, in the accent: it shows only after [`SPINNER_DELAY`],
/// so a trailer that starts at once does not flash it.
pub fn spinner<'a, M: 'a>(since: Instant) -> Element<'a, M> {
    Element::new(Spinner { since })
}

struct Spinner {
    since: Instant,
}

/// Where the arc starts and how wide it is, in degrees, `elapsed` seconds after the delay:
/// the whole turns in [`SPINNER_TURN`], and in each [`SPINNER_CYCLE`] the head runs ahead
/// and then the tail catches up.
fn arc(elapsed: f32) -> (f32, f32) {
    let cycles = elapsed / SPINNER_CYCLE;
    let done = cycles.floor();
    let phase = cycles - done;
    let (head, tail) = match phase < 0.5 {
        true => (ease(phase * 2.0) * SPINNER_MAX_SWEEP, 0.0),
        false => (
            SPINNER_MAX_SWEEP,
            ease((phase - 0.5) * 2.0) * SPINNER_MAX_SWEEP,
        ),
    };
    let start = elapsed / SPINNER_TURN * 360.0 + done * SPINNER_MAX_SWEEP + tail;

    (start % 360.0, (head - tail).max(SPINNER_MIN_SWEEP))
}

fn ease(t: f32) -> f32 {
    match t < 0.5 {
        true => 4.0 * t * t * t,
        false => 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0,
    }
}

fn shows_spinner(since: Instant, now: Instant) -> bool {
    now.saturating_duration_since(since) >= SPINNER_DELAY
}

impl<M> Widget<M, Theme, iced::Renderer> for Spinner {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(SPINNER_SIZE), Length::Fixed(SPINNER_SIZE))
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, SPINNER_SIZE, SPINNER_SIZE)
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let now = Instant::now();
        if !shows_spinner(self.since, now) {
            return;
        }
        let elapsed = (now - self.since - SPINNER_DELAY).as_secs_f32();
        let (start, sweep) = arc(elapsed);
        let bounds = layout.bounds();
        let center = bounds.center();
        let radius = (SPINNER_SIZE - SPINNER_STROKE) / 2.0;
        let color = theme.extended_palette().primary.base.color;
        // The arc is drawn as round dots close enough to touch, one every 4 degrees.
        let dots = (sweep / 4.0).ceil() as usize;
        for dot in 0..=dots {
            let angle = (start + sweep * dot as f32 / dots as f32 - 90.0).to_radians();
            let at = Point::new(
                center.x + radius * angle.cos() - SPINNER_STROKE / 2.0,
                center.y + radius * angle.sin() - SPINNER_STROKE / 2.0,
            );
            renderer.fill_quad(
                Quad {
                    bounds: Rectangle::new(at, Size::new(SPINNER_STROKE, SPINNER_STROKE)),
                    border: iced::Border::default().rounded(SPINNER_STROKE / 2.0),
                    ..Quad::default()
                },
                color,
            );
        }
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let Event::Window(window::Event::RedrawRequested(now)) = event else {
            return;
        };
        if !viewport.intersects(&layout.bounds()) {
            return;
        }
        match shows_spinner(self.since, *now) {
            true => shell.request_redraw(),
            false => shell.request_redraw_at(RedrawRequest::At(self.since + SPINNER_DELAY)),
        }
    }
}

/// Draws the frame the engine picks at each redraw, and asks for the next redraw itself,
/// so that the view is not rebuilt for every frame.
struct Frames<'a> {
    player: &'a Player,
}

impl<M> Widget<M, Theme, iced::Renderer> for Frames<'_> {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, Fill, Fill)
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let Some(frame) = self.player.frame() else {
            return;
        };
        let bounds = layout.bounds();
        renderer.fill_quad(
            Quad {
                bounds,
                ..Quad::default()
            },
            Color::BLACK,
        );
        let area = fit(bounds, frame.width, frame.height);
        if is_software(renderer) {
            if let Some(handle) = &frame.rgba {
                renderer.draw_image(Image::new(handle.clone()), area, bounds);
            }
        } else {
            let shader = Shader::new(Planes(Arc::clone(&frame), self.player.id));
            let tree = Tree::new(&shader as &dyn Widget<M, Theme, iced::Renderer>);
            let node = layout::Node::new(area.size()).move_to(area.position());
            Widget::<M, Theme, iced::Renderer>::draw(
                &shader,
                &tree,
                renderer,
                theme,
                style,
                Layout::new(&node),
                cursor,
                viewport,
            );
        }
        self.player.drawn(&frame);
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let Event::Window(window::Event::RedrawRequested(now)) = event else {
            return;
        };
        if !viewport.intersects(&layout.bounds()) {
            return;
        }
        let request = redraw(
            paced(is_software(renderer), &self.player.source),
            &self.player.state(),
            self.player.shows_an_old_frame(),
            self.player.next_frame_at(),
            *now,
        );
        if let Some(request) = request {
            shell.request_redraw_at(request);
        }
    }
}

/// The shader program of one frame of the player with that id: three `R8Unorm` planes, I420.
struct Planes(Arc<Frame>, u64);

impl<M> shader::Program<M> for Planes {
    type State = ();
    type Primitive = PlanesPrimitive;

    fn draw(&self, _state: &(), _cursor: mouse::Cursor, _bounds: Rectangle) -> PlanesPrimitive {
        PlanesPrimitive(Arc::clone(&self.0), self.1)
    }
}

struct PlanesPrimitive(Arc<Frame>, u64);

impl std::fmt::Debug for PlanesPrimitive {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlanesPrimitive")
            .field("player", &self.1)
            .field("pts", &self.0.pts)
            .finish()
    }
}

struct Textures {
    width: u32,
    height: u32,
    planes: [wgpu::Texture; 3],
    bind_group: wgpu::BindGroup,
}

/// The textures of one player, so that several players draw at once.
struct Slot {
    textures: Textures,
    uploaded: Option<(u64, u64)>,
    used: Instant,
}

pub struct PlanesPipeline {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    slots: HashMap<u64, Slot>,
}

impl Pipeline for PlanesPipeline {
    fn new(device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let source = SHADER.replace("SRGB", if format.is_srgb() { "true" } else { "false" });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("trailer"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let plane = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("trailer"),
            entries: &[
                plane(0),
                plane(1),
                plane(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("trailer"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("trailer"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("trailer"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });

        PlanesPipeline {
            pipeline,
            layout,
            sampler,
            slots: HashMap::new(),
        }
    }
}

impl PlanesPipeline {
    fn textures(&self, device: &wgpu::Device, width: u32, height: u32) -> Textures {
        let texture = |width, height| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("trailer"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let (chroma_width, chroma_height) = chroma(width, height);
        let planes = [
            texture(width, height),
            texture(chroma_width, chroma_height),
            texture(chroma_width, chroma_height),
        ];
        let views = planes
            .each_ref()
            .map(|plane| plane.create_view(&wgpu::TextureViewDescriptor::default()));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("trailer"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        Textures {
            width,
            height,
            planes,
            bind_group,
        }
    }
}

fn is_stale(used: Instant, now: Instant) -> bool {
    now.saturating_duration_since(used) >= TEXTURES_KEPT
}

fn chroma(width: u32, height: u32) -> (u32, u32) {
    (width.div_ceil(2), height.div_ceil(2))
}

impl Primitive for PlanesPrimitive {
    type Pipeline = PlanesPipeline;

    fn prepare(
        &self,
        pipeline: &mut PlanesPipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _bounds: &Rectangle,
        _viewport: &Viewport,
    ) {
        let frame = &self.0;
        let key = (frame.epoch, frame.pts.to_bits());
        let now = Instant::now();
        pipeline
            .slots
            .retain(|&player, slot| player == self.1 || !is_stale(slot.used, now));
        let fits = pipeline.slots.get(&self.1).is_some_and(|slot| {
            slot.textures.width == frame.width && slot.textures.height == frame.height
        });
        if !fits {
            let textures = pipeline.textures(device, frame.width, frame.height);
            pipeline.slots.insert(
                self.1,
                Slot {
                    textures,
                    uploaded: None,
                    used: now,
                },
            );
        }
        let Some(slot) = pipeline.slots.get_mut(&self.1) else {
            return;
        };
        slot.used = now;
        if slot.uploaded == Some(key) {
            return;
        }
        let textures = &slot.textures;
        let (chroma_width, chroma_height) = chroma(frame.width, frame.height);
        let sizes = [
            (frame.width, frame.height),
            (chroma_width, chroma_height),
            (chroma_width, chroma_height),
        ];
        for ((plane, data), (width, height)) in textures
            .planes
            .iter()
            .zip([&frame.y, &frame.u, &frame.v])
            .zip(sizes)
        {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: plane,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        slot.uploaded = Some(key);
    }

    fn draw(&self, pipeline: &PlanesPipeline, pass: &mut wgpu::RenderPass<'_>) -> bool {
        if let Some(slot) = pipeline.slots.get(&self.1) {
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &slot.textures.bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(height: u32, software: bool) -> Surface {
        Surface { height, software }
    }

    fn height(source: Source) -> u32 {
        match source {
            Source::Dash { height, .. } => height,
            Source::Loop { .. } => 0,
        }
    }

    #[test]
    fn a_loop_redraws_at_the_next_frame_with_wgpu_too() {
        let trailer = Source::Dash {
            manifest_url: "https://a/dash_av1.mpd".to_string(),
            height: 720,
        };
        let description = Source::Loop {
            url: "https://a/extras/b.webm".to_string(),
        };

        assert!(!paced(false, &trailer));
        assert!(paced(true, &trailer));
        assert!(paced(false, &description));
        assert!(paced(true, &description));
    }

    #[test]
    fn wgpu_takes_the_planes_and_the_full_height() {
        let wgpu = surface(1013, false);

        assert_eq!(height(wgpu.source("https://a/dash_av1.mpd")), 1013);
        assert_eq!(
            wgpu.options(true, 0.5),
            Options {
                muted: true,
                volume: 0.5,
                rgba: false
            }
        );
    }

    #[test]
    fn tiny_skia_takes_rgba_images_at_360p_at_most() {
        let software = surface(1013, true);

        assert_eq!(height(software.source("https://a/dash_av1.mpd")), 360);
        assert_eq!(
            height(surface(240, true).source("https://a/dash_av1.mpd")),
            240
        );
        assert_eq!(
            software.options(false, 1.0),
            Options {
                muted: false,
                volume: 1.0,
                rgba: true
            }
        );
    }

    #[test]
    fn the_textures_of_a_player_go_after_5_s_without_drawing() {
        let used = Instant::now();

        assert!(!is_stale(used, used + Duration::from_millis(4900)));
        assert!(is_stale(used, used + TEXTURES_KEPT));
    }

    #[test]
    fn the_spinner_waits_300_ms() {
        let since = Instant::now();

        assert!(!shows_spinner(since, since));
        assert!(!shows_spinner(since, since + Duration::from_millis(299)));
        assert!(shows_spinner(since, since + Duration::from_millis(300)));
        assert!(!shows_spinner(since + Duration::from_millis(10), since));
    }

    #[test]
    fn the_spinner_arc_grows_then_shrinks() {
        let (_, short) = arc(0.0);
        let (_, long) = arc(SPINNER_CYCLE * 0.5);
        let (_, back) = arc(SPINNER_CYCLE * 0.99);

        assert_eq!(short, SPINNER_MIN_SWEEP);
        assert_eq!(long, SPINNER_MAX_SWEEP);
        assert!(back < 20.0);
        for step in 0..100 {
            let (start, sweep) = arc(step as f32 * 0.05);
            assert!((0.0..360.0).contains(&start));
            assert!((SPINNER_MIN_SWEEP..=SPINNER_MAX_SWEEP).contains(&sweep));
        }
    }

    #[test]
    fn the_height_is_measured_in_physical_pixels() {
        assert_eq!(physical(405.0, 1.0), 405);
        assert_eq!(physical(405.0, 1.5), 608);
        assert_eq!(physical(-1.0, 2.0), 0);
    }

    #[test]
    fn wgpu_redraws_at_every_vsync_while_the_trailer_moves() {
        let now = Instant::now();
        let at = Some(now + Duration::from_millis(16));

        assert_eq!(
            redraw(false, &State::Playing, false, at, now),
            Some(RedrawRequest::NextFrame)
        );
        assert_eq!(
            redraw(false, &State::Loading, false, None, now),
            Some(RedrawRequest::NextFrame)
        );
    }

    #[test]
    fn tiny_skia_redraws_at_the_next_frame() {
        let now = Instant::now();
        let next = now + Duration::from_millis(16);

        assert_eq!(
            redraw(true, &State::Playing, false, Some(next), now),
            Some(RedrawRequest::At(next))
        );
        assert_eq!(
            redraw(true, &State::Loading, false, None, now),
            Some(RedrawRequest::At(now + IDLE))
        );
    }

    #[test]
    fn nothing_redraws_while_the_trailer_stands_still() {
        let now = Instant::now();
        for state in [
            State::Paused,
            State::Ended,
            State::Failed("no audio device".to_string()),
        ] {
            assert_eq!(redraw(false, &state, false, None, now), None);
            assert_eq!(redraw(true, &state, false, None, now), None);
        }
    }

    #[test]
    fn a_seek_in_pause_redraws_until_the_new_frame_comes() {
        let now = Instant::now();

        assert_eq!(
            redraw(false, &State::Paused, true, None, now),
            Some(RedrawRequest::NextFrame)
        );
        assert_eq!(
            redraw(true, &State::Paused, true, None, now),
            Some(RedrawRequest::At(now + IDLE))
        );
    }

    #[test]
    fn the_frame_keeps_its_shape() {
        let bounds = Rectangle::new(Point::new(10.0, 20.0), Size::new(400.0, 300.0));

        assert_eq!(
            fit(bounds, 1920, 1080),
            Rectangle::new(Point::new(10.0, 57.5), Size::new(400.0, 225.0))
        );
        assert_eq!(
            fit(bounds, 300, 300),
            Rectangle::new(Point::new(60.0, 20.0), Size::new(300.0, 300.0))
        );
        assert_eq!(fit(bounds, 0, 0), bounds);
    }

    #[test]
    fn the_time_reads_minutes_and_seconds() {
        assert_eq!(
            clock(Duration::from_secs(65), Duration::from_secs(85)),
            "1:05 / 1:25"
        );
        assert_eq!(
            clock(Duration::from_millis(999), Duration::from_secs(600)),
            "0:00 / 10:00"
        );
    }
}

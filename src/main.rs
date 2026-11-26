mod profiles;
mod ssh;

use std::path::PathBuf;

use portix_ui::{self as ui, Icon, Palette, gpui};
use gpui::{
    App, Bounds, Context, FocusHandle, FontWeight, KeyBinding, ScrollHandle, Subscription,
    TitlebarOptions, Window, WindowBounds, WindowControlArea, WindowOptions, actions, div,
    point, prelude::*, px, rgb, size,
};
use profiles::Profile;

actions!(portix, [Left, Right, Up, Down, Tab, TabPrev, Quit]);

const TITLE: &str = "SSH profiles";

/// Views that exist in the Flutter app but not in this build yet: shown on the
/// rail so the layout matches, disabled so they never open an empty screen.
const NOT_PORTED: [(Icon, &str); 5] = [
    (Icon::Terminal, "Terminals (none open)"),
    (Icon::Computer, "RDP (not in this build yet)"),
    (Icon::Cable, "SFTP (not in this build yet)"),
    (Icon::Cube, "Kubernetes (not in this build yet)"),
    (Icon::ManageSearch, "Kibana (not in this build yet)"),
];

struct Workspace {
    grid_focus: FocusHandle,
    store: PathBuf,
    profiles: Vec<Profile>,
    load_error: Option<String>,
    /// `None` shows all profiles.
    group: Option<String>,
    /// Index into the visible (filtered) profiles.
    selected: usize,
    /// Grid columns at the last render, for Up/Down.
    cols: usize,
    panel_collapsed: bool,
    scroll: ScrollHandle,
    _appearance: Subscription,
}

impl Workspace {
    fn new(store: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (profiles, load_error) = match profiles::load(&store) {
            Ok(list) => (list, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let grid_focus = cx.focus_handle().tab_stop(true);
        window.focus(&grid_focus);
        Self {
            grid_focus,
            store,
            profiles,
            load_error,
            group: None,
            selected: 0,
            cols: 1,
            panel_collapsed: false,
            scroll: ScrollHandle::new(),
            _appearance: cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
        }
    }

    fn visible(&self) -> Vec<&Profile> {
        self.profiles
            .iter()
            .filter(|p| self.group.as_ref().is_none_or(|g| &p.group == g))
            .collect()
    }

    /// Groups in first-seen order, with their profile counts.
    fn groups(&self) -> Vec<(String, usize)> {
        let mut groups: Vec<(String, usize)> = Vec::new();
        for profile in &self.profiles {
            match groups.iter_mut().find(|(g, _)| *g == profile.group) {
                Some((_, n)) => *n += 1,
                None => groups.push((profile.group.clone(), 1)),
            }
        }
        groups
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.visible().len();
        if n == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).clamp(0, n as isize - 1) as usize;
        self.scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    fn set_group(&mut self, group: Option<String>, cx: &mut Context<Self>) {
        self.group = group;
        self.selected = 0;
        cx.notify();
    }

    fn side_panel(&self, p: &'static Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let tile = |id: usize, icon, label: String, count, group: Option<String>, cx: &mut Context<Self>| {
            let selected = self.group == group;
            ui::side_panel_tile(p, ("group", id), icon, label, Some(count), selected)
                .on_click(cx.listener(move |this, _, _, cx| this.set_group(group.clone(), cx)))
        };
        ui::side_panel(p, TITLE).child(
            div()
                .flex()
                .flex_col()
                .child(tile(0, Icon::Server, "All profiles".into(), self.profiles.len(), None, cx))
                .children(self.groups().into_iter().enumerate().map(|(ix, (group, count))| {
                    tile(ix + 1, Icon::Folder, group.clone(), count, Some(group), cx)
                })),
        )
    }

    fn grid(&self, p: &'static Palette, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(err) = &self.load_error {
            return ui::message(
                p,
                format!("Could not read {}", self.store.display()),
                format!("{err}. Fix or move the file, then reopen Portix."),
            )
            .into_any_element();
        }
        let visible = self.visible();
        if visible.is_empty() {
            return ui::message(
                p,
                "No SSH profiles yet",
                format!("Profiles saved to {} show up here.", self.store.display()),
            )
            .into_any_element();
        }

        let focused = self.grid_focus.is_focused(window);
        div()
            .id("profile-grid")
            .key_context("ProfileGrid")
            .track_focus(&self.grid_focus)
            .on_action(cx.listener(|this, _: &Left, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &Right, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &Up, _, cx| this.step(-(this.cols as isize), cx)))
            .on_action(cx.listener(|this, _: &Down, _, cx| this.step(this.cols as isize, cx)))
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .p_4()
            .grid()
            .grid_cols(self.cols as u16)
            .gap_3()
            .content_start()
            .children(visible.into_iter().enumerate().map(|(ix, profile)| {
                let selected = ix == self.selected;
                ui::card(p, ("profile", ix), selected)
                    .h(px(108.))
                    .flex()
                    .flex_col()
                    .justify_between()
                    .p_3()
                    // Keyboard focus thickens the selected card's accent border.
                    .when(selected && focused, |c| c.border_2().p(px(11.)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(ui::server_icon(profile_color(p, &profile.color), 32.))
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(profile.name.clone()),
                            ),
                    )
                    .child(div().flex().child(ui::pill(p.muted, profile.group.clone())))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.focus(&this.grid_focus);
                        this.selected = ix;
                        cx.notify();
                    }))
            }))
            .into_any_element()
    }
}

/// Profile colors label profiles; they are never used for interaction.
fn profile_color(p: &Palette, name: &str) -> u32 {
    match name {
        "cyan" => p.cyan,
        "blue" => p.muted,
        "pink" => p.danger,
        "amber" => p.amber,
        _ => p.green,
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = ui::palette(window);
        let width = f32::from(window.viewport_size().width);
        let show_panel = !self.panel_collapsed && width >= ui::SIDE_PANEL_MIN_WINDOW_WIDTH;
        let content_width = width - ui::RAIL_WIDTH - if show_panel { ui::SIDE_PANEL_WIDTH } else { 0. } - 32.;
        // Cards stay roughly 300-400px wide, as in the Flutter gallery.
        self.cols = ((content_width / 320.) as usize).clamp(1, 6);

        div()
            .on_action(|_: &Tab, window, _| window.focus_next())
            .on_action(|_: &TabPrev, window, _| window.focus_prev())
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(p.bg))
            .text_color(rgb(p.text))
            .child(ui::top_bar(p, TITLE).window_control_area(WindowControlArea::Drag))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        ui::rail(p)
                            .child(
                                ui::rail_item(p, "rail-profiles", Icon::List, TITLE, true, true)
                                    // Reselecting the active view toggles its side panel.
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.panel_collapsed = !this.panel_collapsed;
                                        cx.notify();
                                    })),
                            )
                            .children(NOT_PORTED.iter().enumerate().map(|(ix, (icon, label))| {
                                ui::rail_item(p, ("rail", ix), *icon, *label, false, false)
                            })),
                    )
                    .when(show_panel, |row| row.child(self.side_panel(p, cx)))
                    .child(div().flex_1().min_w_0().child(self.grid(p, window, cx))),
            )
    }
}

fn main() {
    let store = std::env::home_dir()
        .unwrap_or_default()
        .join(".portix/profiles.json");

    ui::application().run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("left", Left, Some("ProfileGrid")),
            KeyBinding::new("right", Right, Some("ProfileGrid")),
            KeyBinding::new("up", Up, Some("ProfileGrid")),
            KeyBinding::new("down", Down, Some("ProfileGrid")),
            KeyBinding::new("tab", Tab, None),
            KeyBinding::new("shift-tab", TabPrev, None),
            KeyBinding::new("secondary-q", Quit, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_window_closed(|cx| cx.quit()).detach();

        let bounds = Bounds::centered(None, size(px(1200.), px(760.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(TITLE.into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(12.), px(11.))),
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Workspace::new(store, window, cx)),
        )
        .expect("failed to open the Portix window");
        cx.activate(true);
    });
}

use std::collections::HashSet;
use std::fmt::Display;
use std::ops::Deref;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::style::Style;
use ratatui::text::Text;
use ratatui::widgets::{Borders, ListItem, Paragraph, Widget};

use dtu::db::device::models::{
    Apk, DiffSource, DiffedActivity, DiffedApk, DiffedProvider, DiffedReceiver, DiffedService,
    DiffedSystemService, DiffedSystemServiceMethod, SystemService,
};
use dtu::db::{ApkIPC, DeviceDatabase, DeviceSqliteDatabase, Idable};
use dtu::Context;

use crate::diff::ui::customizer::{
    ApkIPCCustomizer, Customizer, ProviderCustomizer, SystemServiceMethodCustomizer,
};
use crate::diff::ui::filter_boxes::{ApkIPCFilterBox, SystemServiceMethodFilterBox};
use crate::diff::ui::tabs::{Tab, TabContainer};
use crate::diff::ui::ui::{ActiveSection, ActiveTab};
use crate::ui::widgets::{BlockBuilder, ClosureWidget, BG_COLOR, FG_COLOR};

use super::state::State;

pub struct Applet<'a> {
    active_tab: Box<dyn Tab>,
    ctx: &'a dyn Context,
    db: DeviceSqliteDatabase,
    active_section: ActiveSection,
    diff_source: DiffSource,
    state: State,
    search_string: Option<String>,
    editing_search_string: bool,
    showing_popup: bool,
    showing_help: bool,
    show_hidden: bool,
    should_quit: bool,
}

impl<'a> Applet<'a> {
    pub fn new(ctx: &'a dyn Context, diff_source: DiffSource) -> anyhow::Result<Self> {
        let db = DeviceSqliteDatabase::new(ctx)?;
        let state = State::load(ctx)?;
        let active_tab =
            system_services_tab(&db, diff_source.id, &state.hidden_system_services, true)?;
        Ok(Self {
            ctx,
            db,
            diff_source,
            state,
            active_tab,
            active_section: ActiveSection::ItemList,
            editing_search_string: false,
            search_string: None,
            should_quit: false,
            showing_help: false,
            show_hidden: true,
            showing_popup: false,
        })
    }

    pub fn get_editing_search_string(&self) -> bool {
        self.editing_search_string
    }

    pub fn get_search_string(&self) -> Option<String> {
        if self.search_string.is_none() && self.editing_search_string {
            return Some(String::new());
        }
        self.search_string.clone()
    }

    pub fn get_selection_idx(&self) -> usize {
        self.active_tab.get_selected_idx()
    }

    pub fn get_active_items_list(&self) -> Vec<ListItem<'_>> {
        self.active_tab.get_list_items()
    }

    pub fn get_active_tab(&self) -> ActiveTab {
        self.active_tab.get_type()
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    fn on_key_event_with_filter_box(&mut self, evt: KeyEvent) -> bool {
        match evt.modifiers {
            KeyModifiers::CONTROL => match evt.code {
                KeyCode::Char('w') => {
                    self.active_section = self.active_section.next();
                    return true;
                }
                KeyCode::Char('c') => {
                    self.on_quit();
                    return true;
                }
                _ => {}
            },
            _ => {}
        }

        match self.active_section {
            ActiveSection::ItemList => self.on_list_focused_key_event(evt),
            ActiveSection::FilterBox => self.active_tab.on_filter_box_key_event(evt),
        }
    }

    pub fn on_key_event(&mut self, evt: KeyEvent) -> bool {
        if self.active_tab.has_filter_box() {
            return self.on_key_event_with_filter_box(evt);
        }
        self.on_list_focused_key_event(evt)
    }

    fn on_list_focused_key_event(&mut self, evt: KeyEvent) -> bool {
        match evt.modifiers {
            KeyModifiers::CONTROL => self.on_ctrl_key_event(evt.code),
            KeyModifiers::SHIFT => self.on_shift_key_event(evt.code),
            KeyModifiers::NONE => self.on_unmodified_key_event(evt.code),
            _ => false,
        }
    }

    fn on_shift_key_event(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Char(c) if self.editing_search_string => self.update_search_string(c),
            KeyCode::Char('J') => self.next_tab(),
            KeyCode::Char('K') => self.prev_tab(),
            KeyCode::Char('H') => self.toggle_active_selection_hidden(),
            KeyCode::Char('G') => self.sel_end(),
            KeyCode::Char('O') => self.open_requested(),
            _ => return false,
        }
        true
    }

    fn on_unmodified_key_event(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Char(c) => return self.on_unmodified_char_key(c),
            KeyCode::Backspace if self.editing_search_string => self.search_string_del(),
            // Keep this for historical reasons
            KeyCode::Tab => self.on_unmodified_tab(),
            KeyCode::Enter if self.editing_search_string => self.editing_search_string = false,
            KeyCode::Enter => self.showing_popup = !self.showing_popup,
            _ => return false,
        }
        true
    }

    fn on_unmodified_tab(&mut self) {
        self.next_tab()
    }

    fn clipboard_requested(&self) {
        if let Err(e) = self.active_tab.clipboard_selection(self.ctx) {
            log::error!("failed to clipboard selection: {}", e);
        }
    }

    fn open_requested(&self) {
        if let Err(e) = self.active_tab.open_selection(self.ctx) {
            log::error!("failed to open selection: {}", e);
        }
    }

    fn search_string_del(&mut self) {
        match self.search_string.as_ref().map(|it| it.len()) {
            None => return,
            Some(v) if v <= 1 => self.search_string = None,
            Some(_) => {
                self.search_string.as_mut().map(|it| {
                    let _ = it.pop();
                });
            }
        }
        self.active_tab
            .set_search_string(self.search_string.clone());
    }

    fn update_search_string(&mut self, c: char) {
        if self.search_string.is_none() {
            self.search_string = Some(String::from(c));
        } else {
            self.search_string.as_mut().map(|it| {
                it.push(c);
            });
        }
        self.active_tab
            .set_search_string(self.search_string.clone());
    }

    fn on_unmodified_char_key(&mut self, c: char) -> bool {
        if self.editing_search_string {
            self.update_search_string(c);
            return true;
        }
        match c {
            '/' => self.editing_search_string = true,
            '?' => self.toggle_help(),
            '.' => self.toggle_show_hidden(),
            'j' => self.inc_sel_idx(),
            'k' => self.dec_sel_idx(),
            'g' => self.sel_start(),
            'c' => self.clipboard_requested(),
            _ => return false,
        }
        true
    }

    fn toggle_show_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.active_tab.set_show_hidden(self.show_hidden);
    }

    fn on_ctrl_key_event(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Char('c') => self.on_quit(),
            _ => return false,
        }
        true
    }

    pub fn on_mouse_event(&mut self, evt: MouseEvent) -> bool {
        if self.active_section == ActiveSection::FilterBox {
            return self.active_tab.on_filter_box_mouse_event(evt);
        }
        match evt.kind {
            MouseEventKind::ScrollDown => self.inc_sel_idx(),
            MouseEventKind::ScrollUp => self.dec_sel_idx(),
            _ => return false,
        }
        true
    }

    fn on_quit(&mut self) {
        self.update_state();
        if let Err(e) = self.state.store(&self.ctx) {
            log::error!("saving state: {}", e);
        }
        self.should_quit = true;
    }

    fn dec_sel_idx(&mut self) {
        self.active_tab.dec_selection();
    }

    fn inc_sel_idx(&mut self) {
        self.active_tab.inc_selection();
    }

    fn sel_end(&mut self) {
        self.active_tab.select_end();
    }

    fn sel_start(&mut self) {
        self.active_tab.select_start();
    }

    fn toggle_help(&mut self) {
        self.showing_help = !self.showing_help
    }

    fn toggle_active_selection_hidden(&mut self) {
        self.active_tab.toggle_selected_hidden()
    }

    pub fn get_filter_widget(&self) -> Option<ClosureWidget> {
        self.active_tab.get_filter_box()
    }

    fn update_state(&mut self) {
        let set = self.active_tab.get_hidden_set();
        let state = &mut self.state;
        match self.active_tab.get_type() {
            ActiveTab::SystemServices => state.hidden_system_services = set,
            ActiveTab::SystemServiceMethods => state.hidden_system_service_methods = set,
            ActiveTab::Apks => state.hidden_apks = set,
            ActiveTab::Receivers => state.hidden_receivers = set,
            ActiveTab::Providers => state.hidden_providers = set,
            ActiveTab::Services => state.hidden_services = set,
            ActiveTab::Activities => state.hidden_activities = set,
        }
    }

    fn prev_tab(&mut self) {
        self.update_state();
        let res = match self.active_tab.get_type() {
            ActiveTab::SystemServices => self.get_activities_tab(),
            ActiveTab::SystemServiceMethods => self.get_system_services_tab(),
            ActiveTab::Apks => self.get_system_service_methods_tab(),
            ActiveTab::Providers => self.get_apks_tab(),
            ActiveTab::Receivers => self.get_providers_tab(),
            ActiveTab::Services => self.get_receivers_tab(),
            ActiveTab::Activities => self.get_services_tab(),
        };

        let tab = match res {
            Err(e) => {
                log::error!("changing tab: {}", e);
                return;
            }
            Ok(v) => v,
        };
        self.active_tab = tab;
    }

    fn next_tab(&mut self) {
        self.update_state();
        let res = match self.active_tab.get_type() {
            ActiveTab::SystemServices => self.get_system_service_methods_tab(),
            ActiveTab::SystemServiceMethods => self.get_apks_tab(),
            ActiveTab::Apks => self.get_providers_tab(),
            ActiveTab::Providers => self.get_receivers_tab(),
            ActiveTab::Receivers => self.get_services_tab(),
            ActiveTab::Services => self.get_activities_tab(),
            ActiveTab::Activities => self.get_system_services_tab(),
        };

        let tab = match res {
            Err(e) => {
                log::error!("changing tab: {}", e);
                return;
            }
            Ok(v) => v,
        };
        self.active_tab = tab;
    }

    fn get_system_service_methods_tab(&self) -> anyhow::Result<Box<dyn Tab>> {
        let methods = self
            .db
            .get_system_service_method_diffs_by_diff_id(self.diff_source.id)?
            .iter()
            .filter(|it| !it.exists_in_diff || it.hash_matches_diff.is_false())
            .map(|it| it.clone())
            .collect::<Vec<DiffedSystemServiceMethod>>();
        let mut service_ids = HashSet::new();
        service_ids.extend(methods.iter().map(|it| it.system_service_id));
        let hidden_services = self.state.hidden_system_services.clone();
        let services = self
            .db
            .get_system_services()
            .ok()
            .unwrap_or(vec![])
            .iter()
            .filter(|it| service_ids.contains(&it.id))
            .map(|it| it.clone())
            .collect::<Vec<SystemService>>();

        let container = self.new_tab_container(
            methods,
            self.state.hidden_system_service_methods.clone(),
            ActiveTab::SystemServiceMethods,
            Some(Box::new(SystemServiceMethodCustomizer::new(
                self.db.clone(),
                hidden_services,
            ))),
            Some(Box::new(SystemServiceMethodFilterBox::new(services))),
        );

        Ok(container)
    }

    fn get_apks_tab(&self) -> anyhow::Result<Box<dyn Tab>> {
        let apks = self
            .db
            .get_apk_diffs_by_diff_id(self.diff_source.id)?
            .iter()
            .filter(|it| !it.exists_in_diff)
            .map(|it| it.clone())
            .collect::<Vec<DiffedApk>>();
        let container = Box::new(TabContainer::new(
            apks,
            self.state.hidden_apks.clone(),
            ActiveTab::Apks,
            self.show_hidden,
        ));

        Ok(container)
    }

    fn new_apk_ipc_customizer<U, T>(&self) -> Box<ApkIPCCustomizer<U>>
    where
        T: ApkIPC + Display,
        U: Deref<Target = T>,
    {
        Box::new(ApkIPCCustomizer::new(
            self.db.clone(),
            self.state.hidden_apks.clone(),
        ))
    }

    fn new_apk_ipc_filter<U, T>(&self, items: &[U]) -> Box<ApkIPCFilterBox<U>>
    where
        T: ApkIPC + Display,
        U: Deref<Target = T>,
    {
        let mut apk_ids = HashSet::new();
        apk_ids.extend(items.iter().map(|it| it.get_apk_id()));
        let normal_perms = self
            .db
            .get_normal_permissions()
            .ok()
            .unwrap_or(vec![])
            .iter()
            .map(|it| it.name.clone())
            .collect::<Vec<String>>();
        let apks = self
            .db
            .get_apks()
            .ok()
            .unwrap_or(vec![])
            .iter()
            .filter(|it| apk_ids.contains(&it.id))
            .map(|it| it.clone())
            .collect::<Vec<Apk>>();
        Box::new(ApkIPCFilterBox::new(apks, normal_perms))
    }

    fn new_tab_container<E>(
        &self,
        items: Vec<E>,
        hidden_list: HashSet<i32>,
        tab_type: ActiveTab,
        customizer: Option<Box<dyn Customizer<E>>>,
        filter_box: Option<Box<dyn FilterBox<E>>>,
    ) -> Box<TabContainer<E>>
    where
        E: Display + Idable,
    {
        let mut container = Box::new(TabContainer::new(
            items,
            hidden_list.clone(),
            tab_type,
            self.show_hidden,
        ));
        container.customizer = customizer;
        container.set_filter_box(filter_box);
        container
    }

    fn get_receivers_tab(&self) -> anyhow::Result<Box<dyn Tab>> {
        let receivers = self
            .db
            .get_receiver_diffs_by_diff_id(self.diff_source.id)?
            .iter()
            .filter(|it| !it.exists_in_diff && it.enabled && it.exported)
            .map(|it| it.clone())
            .collect::<Vec<DiffedReceiver>>();

        let customizer = self.new_apk_ipc_customizer();
        let filter = self.new_apk_ipc_filter(receivers.as_slice());

        Ok(self.new_tab_container(
            receivers,
            self.state.hidden_receivers.clone(),
            ActiveTab::Receivers,
            Some(customizer),
            Some(filter),
        ))
    }

    fn get_services_tab(&self) -> anyhow::Result<Box<dyn Tab>> {
        let services = self
            .db
            .get_service_diffs_by_diff_id(self.diff_source.id)?
            .iter()
            .filter(|it| !it.exists_in_diff && it.enabled && it.exported)
            .map(|it| it.clone())
            .collect::<Vec<DiffedService>>();

        let customizer = self.new_apk_ipc_customizer();
        let mut filter = self.new_apk_ipc_filter(services.as_slice());
        filter.add_custom(
            "Returns Binder".into(),
            Arc::new(|it| it.service.returns_binder.is_false()),
        );

        Ok(self.new_tab_container(
            services,
            self.state.hidden_services.clone(),
            ActiveTab::Services,
            Some(customizer),
            Some(filter),
        ))
    }

    fn get_activities_tab(&self) -> anyhow::Result<Box<dyn Tab>> {
        let activities = self
            .db
            .get_activity_diffs_by_diff_id(self.diff_source.id)?
            .iter()
            .filter(|it| !it.exists_in_diff && it.enabled && it.exported)
            .map(|it| it.clone())
            .collect::<Vec<DiffedActivity>>();
        let customizer = self.new_apk_ipc_customizer();
        let filter = self.new_apk_ipc_filter(activities.as_slice());

        Ok(self.new_tab_container(
            activities,
            self.state.hidden_activities.clone(),
            ActiveTab::Activities,
            Some(customizer),
            Some(filter),
        ))
    }

    fn get_providers_tab(&self) -> anyhow::Result<Box<dyn Tab>> {
        let providers = self
            .db
            .get_provider_diffs_by_diff_id(self.diff_source.id)?
            .iter()
            .filter(|it| !it.exists_in_diff && it.enabled && it.exported)
            .map(|it| it.clone())
            .collect::<Vec<DiffedProvider>>();
        let filter_box = self.new_apk_ipc_filter(providers.as_slice());
        let customizer = Box::new(ProviderCustomizer::new(
            self.db.clone(),
            self.state.hidden_apks.clone(),
        ));
        let mut container = Box::new(TabContainer::new(
            providers,
            self.state.hidden_providers.clone(),
            ActiveTab::Providers,
            self.show_hidden,
        ));
        container.filter_box = Some(filter_box);
        container.customizer = Some(customizer);
        Ok(container)
    }

    fn get_system_services_tab(&self) -> anyhow::Result<Box<dyn Tab>> {
        let tab = system_services_tab(
            &self.db,
            self.diff_source.id,
            &self.state.hidden_system_services,
            self.show_hidden,
        )?;
        Ok(tab)
    }

    pub fn get_popup(&self) -> Option<ClosureWidget> {
        if self.showing_help {
            Some(self.get_help_popup())
        } else if self.showing_popup {
            self.active_tab.get_info_popup()
        } else {
            None
        }
    }

    fn get_help_popup(&self) -> ClosureWidget {
        ClosureWidget::new(Box::new(move |area, buf| {
            let widget = Paragraph::new(Text::from(HELP_TEXT))
                .style(Style::default().fg(BG_COLOR))
                .block(
                    BlockBuilder::default()
                        .with_borders(Borders::ALL)
                        .with_style(Style::default().bg(FG_COLOR))
                        .build(),
                );
            widget.render(area, buf);
        }))
    }
}

/// Function used to filter out a given item from the display
///
/// This function should return `true` if the item should be hidden and false
/// otherwise
pub type FilterBoxFunction<T> = dyn Fn(&T) -> bool;

/// Used by the UI to display a filter box.
pub trait FilterBox<E> {
    fn on_key_event(&mut self, evt: KeyEvent) -> bool;
    fn on_mouse_event(&mut self, evt: MouseEvent) -> bool;
    /// Make a filter to say whether a given item should be displayed or not
    ///
    /// Return true if the item should be filtered and false otherwise
    fn make_filter(&self) -> Option<Box<FilterBoxFunction<E>>>;
    fn get_widget(&self) -> Option<ClosureWidget>;
}

fn system_services_tab(
    db: &dyn DeviceDatabase,
    diff_id: i32,
    hidden_list: &HashSet<i32>,
    show_hidden: bool,
) -> anyhow::Result<Box<dyn Tab>> {
    let items = db
        .get_system_service_diffs_by_diff_id(diff_id)?
        .iter()
        .filter(|it| !it.exists_in_diff)
        .map(|it| it.clone())
        .collect::<Vec<DiffedSystemService>>();

    Ok(Box::new(TabContainer::new(
        items,
        hidden_list.clone(),
        ActiveTab::SystemServices,
        show_hidden,
    )))
}

static HELP_TEXT: &'static str = r#"== Help ==

Note that these might have different meanings if the
filter box is focused (Ctrl + W)

Enter           Show info for highlighted

Ctrl + W        Focus/unfocus filter box
O               Attempt to open tile file containing the selection
                with $DTU_OPEN_EXECUTABLE or dtu-open-file
c               Invoke the clipboard action with $DTU_CLIPBOARD_EXECUTABLE
                or dtu-clipboard
J/K             Change diff type down/up
j/k             Move selection down/up
g               Move selection to top
G               Move selection to bottom

H               Toggle selection hidden
.               Toggle show hidden
/               Search in list (regex supported)
?               Show/hide help

Ctrl + C        quit
"#;

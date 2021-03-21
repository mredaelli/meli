/*
 * meli
 *
 * Copyright 2017-2018 Manos Pitsidianakis
 *
 * This file is part of meli.
 *
 * meli is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * meli is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with meli. If not, see <http://www.gnu.org/licenses/>.
 */

/*! The application's state.

The UI crate has an Box<dyn Component>-Component-System design. The System part, is also the application's state, so they're both merged in the `State` struct.

`State` owns all the Components of the UI. In the application's main event loop, input is handed to the state in the form of `UIEvent` objects which traverse the component graph. Components decide to handle each input or not.

Input is received in the main loop from threads which listen on the stdin for user input, observe folders for file changes etc. The relevant struct is `ThreadEvent`.
*/

use super::*;
//use crate::plugins::PluginManager;
use melib::backends::{AccountHash, BackendEventConsumer};

use crate::jobs::JobExecutor;
use crossbeam::channel::{after, unbounded, Receiver, Sender};
use indexmap::IndexMap;
use smallvec::SmallVec;
use std::env;
use std::io::Write;
use std::os::unix::io::RawFd;
use std::sync::Arc;
use std::thread;
use termion::raw::IntoRawMode;
use termion::screen::AlternateScreen;
use termion::{clear, cursor};

pub type StateStdout = termion::screen::AlternateScreen<termion::raw::RawTerminal<std::io::Stdout>>;

struct InputHandler {
    pipe: (RawFd, RawFd),
    rx: Receiver<InputCommand>,
    tx: Sender<InputCommand>,
    state_tx: Sender<ThreadEvent>,
    control: std::sync::Weak<()>,
    bindings: Bindings,
    chord_timeout_ms: u32,
}

use std::time::{Duration, Instant};
#[derive(Debug, Clone)]
struct BindingHandler {
    last_ts: Instant,
    timeoutlen: Duration,
    tx: Sender<ThreadEvent>,
    bindings: Bindings,
    prefix: Vec<(Key, Vec<u8>)>,
}
impl BindingHandler {
    pub fn new(tx: Sender<ThreadEvent>, bindings: Bindings, chord_timeout_ms: u32) -> Self {
        BindingHandler {
            tx,
            last_ts: Instant::now(),
            timeoutlen: Duration::new(0, chord_timeout_ms * 1000 * 1000),
            bindings: bindings.clone(),
            prefix: vec![],
        }
    }
    pub fn handle_input(&mut self, key: Key, x: Vec<u8>) -> bool {
        if Instant::now() - self.last_ts > self.timeoutlen && !self.prefix.is_empty() {
            // No input for a while, but some keys left over: just send what we have
            for key in &self.prefix {
                self.tx.send(ThreadEvent::Input(key.clone())).unwrap();
            }
            self.prefix = vec![];
            // and proceed normally
        }

        self.prefix.push((key.clone(), x.clone()));
        let filtered_bindings = filter(&self.bindings.normal, &self.prefix);
        let need_to_wait = match filtered_bindings.len() {
            0 => {
                // No matching macro: send the keys
                for key in &self.prefix {
                    self.tx.send(ThreadEvent::Input(key.clone())).unwrap();
                }
                self.prefix = vec![];
                false
            }
            1 => {
                let (keys, cmd) = filtered_bindings.into_iter().next().unwrap();
                if self.prefix.len() == keys.len() {
                    // Exact match: send the command
                    // TODO: if we want them recursive... not sure
                    // TODO: decoding special characters
                    for key in cmd.chars() {
                        self.tx
                            .send(ThreadEvent::Input((Key::Char(key), vec![])))
                            .unwrap();
                    }
                    self.prefix = vec![];
                    false
                } else {
                    true
                }
            }
            _ => true,
        };
        if need_to_wait {
            self.last_ts = Instant::now();
        }
        need_to_wait
    }
}

impl InputHandler {
    fn restore(&mut self) {
        let working = Arc::new(());
        let control = Arc::downgrade(&working);

        /* Clear channel without blocking. switch_to_main_screen() issues a kill when
         * returning from a fork and there's no input thread, so the newly created thread will
         * receive it and die. */
        //let _ = self.rx.try_iter().count();
        let rx = self.rx.clone();
        let pipe = self.pipe.0;
        let tx = self.state_tx.clone();
        let bindings = self.bindings.clone();
        let timeout = self.chord_timeout_ms.clone();
        thread::Builder::new()
            .name("input-thread".to_string())
            .spawn(move || {
                let mut h = BindingHandler::new(tx.clone(), bindings, timeout);
                get_events(|(k, x)| h.handle_input(k, x), &rx, pipe, working, timeout)
            })
            .unwrap();
        self.control = control;
    }

    fn kill(&self) {
        let _ = nix::unistd::write(self.pipe.1, &[1]);
        self.tx.send(InputCommand::Kill).unwrap();
    }

    fn check(&mut self) {
        match self.control.upgrade() {
            Some(_) => {}
            None => {
                debug!("restarting input_thread");
                self.restore();
            }
        }
    }
}

/// A context container for loaded settings, accounts, UI changes, etc.
pub struct Context {
    pub accounts: IndexMap<AccountHash, Account>,
    pub settings: Settings,

    /// Areas of the screen that must be redrawn in the next render
    pub dirty_areas: VecDeque<Area>,

    /// Events queue that components send back to the state
    pub replies: VecDeque<UIEvent>,
    pub sender: Sender<ThreadEvent>,
    receiver: Receiver<ThreadEvent>,
    input_thread: InputHandler,
    pub job_executor: Arc<JobExecutor>,
    pub children: Vec<std::process::Child>,

    pub temp_files: Vec<File>,
}

impl Context {
    pub fn replies(&mut self) -> smallvec::SmallVec<[UIEvent; 8]> {
        self.replies.drain(0..).collect()
    }

    pub fn input_kill(&self) {
        self.input_thread.kill();
    }

    pub fn restore_input(&mut self) {
        self.input_thread.restore();
    }

    pub fn is_online_idx(&mut self, account_pos: usize) -> Result<()> {
        let Context {
            ref mut accounts,
            ref mut replies,
            ..
        } = self;
        let was_online = accounts[account_pos].is_online.is_ok();
        let ret = accounts[account_pos].is_online();
        if ret.is_ok() {
            if !was_online {
                debug!("inserting mailbox hashes:");
                for mailbox_node in accounts[account_pos].list_mailboxes() {
                    debug!(
                        "hash & mailbox: {:?} {}",
                        mailbox_node.hash,
                        accounts[account_pos][&mailbox_node.hash].name()
                    );
                }
                accounts[account_pos].watch();

                replies.push_back(UIEvent::AccountStatusChange(accounts[account_pos].hash()));
            }
        }
        if ret.is_ok() != was_online {
            replies.push_back(UIEvent::AccountStatusChange(accounts[account_pos].hash()));
        }
        ret
    }

    pub fn is_online(&mut self, account_hash: AccountHash) -> Result<()> {
        let idx = self.accounts.get_index_of(&account_hash).unwrap();
        self.is_online_idx(idx)
    }
}

/// A State object to manage and own components and components of the UI. `State` is responsible for
/// managing the terminal and interfacing with `melib`
pub struct State {
    cols: usize,
    rows: usize,

    grid: CellBuffer,
    overlay_grid: CellBuffer,
    draw_rate_limit: RateLimit,
    stdout: Option<StateStdout>,
    mouse: bool,
    child: Option<ForkType>,
    draw_horizontal_segment_fn: fn(&mut CellBuffer, &mut StateStdout, usize, usize, usize) -> (),
    pub mode: UIMode,
    overlay: Vec<Box<dyn Component>>,
    components: Vec<Box<dyn Component>>,
    pub context: Context,
    timer: thread::JoinHandle<()>,

    display_messages: SmallVec<[DisplayMessage; 8]>,
    display_messages_expiration_start: Option<UnixTimestamp>,
    display_messages_active: bool,
    display_messages_dirty: bool,
    display_messages_initialised: bool,
    display_messages_pos: usize,
    display_messages_area: Area,
}

#[derive(Debug)]
struct DisplayMessage {
    timestamp: UnixTimestamp,
    msg: String,
}

impl Drop for State {
    fn drop(&mut self) {
        // When done, restore the defaults to avoid messing with the terminal.
        self.switch_to_main_screen();
        use nix::sys::wait::{waitpid, WaitPidFlag};
        for child in self.context.children.iter_mut() {
            if let Err(err) = waitpid(
                nix::unistd::Pid::from_raw(child.id() as i32),
                Some(WaitPidFlag::WNOHANG),
            ) {
                debug!("Failed to wait on subprocess {}: {}", child.id(), err);
            }
        }
        if let Some(ForkType::Embed(child_pid)) = self.child.take() {
            /* Try wait, we don't want to block */
            if let Err(e) = waitpid(child_pid, Some(WaitPidFlag::WNOHANG)) {
                debug!("Failed to wait on subprocess {}: {}", child_pid, e);
            }
        }
    }
}

impl State {
    pub fn new(
        settings: Option<Settings>,
        sender: Sender<ThreadEvent>,
        receiver: Receiver<ThreadEvent>,
    ) -> Result<Self> {
        /*
         * Create async channel to block the input-thread if we need to fork and stop it from reading
         * stdin, see get_events() for details
         * */
        let input_thread = unbounded();
        let input_thread_pipe = nix::unistd::pipe()
            .map_err(|err| Box::new(err) as Box<dyn std::error::Error + Send + Sync + 'static>)?;
        let backends = Backends::new();
        let settings = if let Some(settings) = settings {
            settings
        } else {
            Settings::new()?
        };
        /*
        let mut plugin_manager = PluginManager::new();
        for (_, p) in settings.plugins.clone() {
            if crate::plugins::PluginKind::Backend == p.kind() {
                debug!("registering {:?}", &p);
                crate::plugins::backend::PluginBackend::register(
                    plugin_manager.listener(),
                    p.clone(),
                    &mut backends,
                );
            }
            plugin_manager.register(p)?;
        }
        */

        let termsize = termion::terminal_size()?;
        let cols = termsize.0 as usize;
        let rows = termsize.1 as usize;

        let job_executor = Arc::new(JobExecutor::new(sender.clone()));
        let accounts = {
            settings
                .accounts
                .iter()
                .map(|(n, a_s)| {
                    let sender = sender.clone();
                    let account_hash = {
                        use std::collections::hash_map::DefaultHasher;
                        use std::hash::Hasher;
                        let mut hasher = DefaultHasher::new();
                        hasher.write(n.as_bytes());
                        hasher.finish()
                    };
                    Account::new(
                        account_hash,
                        n.to_string(),
                        a_s.clone(),
                        &backends,
                        job_executor.clone(),
                        sender.clone(),
                        BackendEventConsumer::new(Arc::new(
                            move |account_hash: AccountHash, ev: BackendEvent| {
                                sender
                                    .send(ThreadEvent::UIEvent(UIEvent::BackendEvent(
                                        account_hash,
                                        ev,
                                    )))
                                    .unwrap();
                            },
                        )),
                    )
                })
                .collect::<Result<Vec<Account>>>()?
        };
        let accounts = accounts.into_iter().map(|acc| (acc.hash(), acc)).collect();

        let timer = {
            let sender = sender.clone();
            thread::Builder::new().spawn(move || {
                let sender = sender;
                loop {
                    thread::park();

                    sender.send(ThreadEvent::Pulse).unwrap();
                    thread::sleep(std::time::Duration::from_millis(100));
                }
            })
        }?;

        timer.thread().unpark();

        let working = Arc::new(());
        let control = Arc::downgrade(&working);
        let bindings = settings.bindings.clone();
        let mut s = State {
            cols,
            rows,
            grid: CellBuffer::new(cols, rows, Cell::with_char(' ')),
            overlay_grid: CellBuffer::new(cols, rows, Cell::with_char(' ')),
            stdout: None,
            mouse: settings.terminal.use_mouse.is_true(),
            child: None,
            mode: UIMode::Normal,
            components: Vec::with_capacity(8),
            overlay: Vec::new(),
            timer,
            draw_rate_limit: RateLimit::new(1, 3, job_executor.clone()),
            draw_horizontal_segment_fn: if settings.terminal.use_color() {
                State::draw_horizontal_segment
            } else {
                State::draw_horizontal_segment_no_color
            },
            display_messages: SmallVec::new(),
            display_messages_expiration_start: None,
            display_messages_pos: 0,
            display_messages_active: false,
            display_messages_dirty: false,
            display_messages_initialised: false,
            display_messages_area: ((0, 0), (0, 0)),
            context: Context {
                accounts,
                settings: settings,
                dirty_areas: VecDeque::with_capacity(5),
                replies: VecDeque::with_capacity(5),
                temp_files: Vec::new(),
                job_executor,
                children: vec![],

                input_thread: InputHandler {
                    pipe: input_thread_pipe,
                    rx: input_thread.1,
                    tx: input_thread.0,
                    control,
                    state_tx: sender.clone(),
                    bindings,
                    chord_timeout_ms: 500u32,
                },
                sender,
                receiver,
            },
        };
        if s.context.settings.terminal.ascii_drawing {
            s.grid.set_ascii_drawing(true);
            s.overlay_grid.set_ascii_drawing(true);
        }

        s.switch_to_alternate_screen();
        for i in 0..s.context.accounts.len() {
            if !s.context.accounts[i].backend_capabilities.is_remote {
                s.context.accounts[i].watch();
            }
            if s.context.is_online_idx(i).is_ok() && s.context.accounts[i].is_empty() {
                //return Err(MeliError::new(format!(
                //    "Account {} has no mailboxes configured.",
                //    s.context.accounts[i].name()
                //)));
            }
        }
        s.context.restore_input();
        Ok(s)
    }

    /*
     * When we receive a mailbox hash from a watcher thread,
     * we match the hash to the index of the mailbox, request a reload
     * and startup a thread to remind us to poll it every now and then till it's finished.
     */
    pub fn refresh_event(&mut self, event: RefreshEvent) {
        let account_hash = event.account_hash;
        let mailbox_hash = event.mailbox_hash;
        if self.context.accounts[&account_hash]
            .mailbox_entries
            .contains_key(&mailbox_hash)
        {
            if self.context.accounts[&account_hash]
                .load(mailbox_hash)
                .is_err()
            {
                self.context.replies.push_back(UIEvent::from(event));
                return;
            }
            let Context {
                ref mut accounts, ..
            } = &mut self.context;

            if let Some(notification) = accounts[&account_hash].reload(event, mailbox_hash) {
                if let UIEvent::Notification(_, _, _) = notification {
                    self.rcv_event(UIEvent::MailboxUpdate((account_hash, mailbox_hash)));
                }
                self.rcv_event(notification);
            }
        } else {
            if let melib::backends::RefreshEventKind::Failure(err) = event.kind {
                debug!(err);
            }
        }
    }

    /// Switch back to the terminal's main screen (The command line the user sees before opening
    /// the application)
    pub fn switch_to_main_screen(&mut self) {
        let mouse = self.mouse;
        write!(
            self.stdout(),
            "{}{}{}{}{disable_sgr_mouse}{disable_mouse}",
            termion::screen::ToMainScreen,
            cursor::Show,
            RestoreWindowTitleIconFromStack,
            BracketModeEnd,
            disable_sgr_mouse = if mouse { DisableSGRMouse.as_ref() } else { "" },
            disable_mouse = if mouse { DisableMouse.as_ref() } else { "" },
        )
        .unwrap();
        self.flush();
        self.stdout = None;
    }

    pub fn switch_to_alternate_screen(&mut self) {
        let s = std::io::stdout();

        let mut stdout = AlternateScreen::from(s.into_raw_mode().unwrap());

        write!(
            &mut stdout,
            "{save_title_to_stack}{}{}{}{window_title}{}{}{enable_mouse}{enable_sgr_mouse}",
            termion::screen::ToAlternateScreen,
            cursor::Hide,
            clear::All,
            cursor::Goto(1, 1),
            BracketModeStart,
            save_title_to_stack = SaveWindowTitleIconToStack,
            window_title = if let Some(ref title) = self.context.settings.terminal.window_title {
                format!("\x1b]2;{}\x07", title)
            } else {
                String::new()
            },
            enable_mouse = if self.mouse { EnableMouse.as_ref() } else { "" },
            enable_sgr_mouse = if self.mouse {
                EnableSGRMouse.as_ref()
            } else {
                ""
            },
        )
        .unwrap();

        self.stdout = Some(stdout);
        self.flush();
    }

    pub fn set_mouse(&mut self, value: bool) {
        if let Some(stdout) = self.stdout.as_mut() {
            write!(
                stdout,
                "{mouse}{sgr_mouse}",
                mouse = if value {
                    AsRef::<str>::as_ref(&EnableMouse)
                } else {
                    AsRef::<str>::as_ref(&DisableMouse)
                },
                sgr_mouse = if value {
                    AsRef::<str>::as_ref(&EnableSGRMouse)
                } else {
                    AsRef::<str>::as_ref(&DisableSGRMouse)
                },
            )
            .unwrap();
        }
        self.flush();
    }

    pub fn receiver(&self) -> Receiver<ThreadEvent> {
        self.context.receiver.clone()
    }

    pub fn sender(&self) -> Sender<ThreadEvent> {
        self.context.sender.clone()
    }

    pub fn restore_input(&mut self) {
        self.context.restore_input();
    }

    /// On `SIGWNICH` the `State` redraws itself according to the new terminal size.
    pub fn update_size(&mut self) {
        let termsize = termion::terminal_size().ok();
        let termcols = termsize.map(|(w, _)| w);
        let termrows = termsize.map(|(_, h)| h);
        if termcols.unwrap_or(72) as usize != self.cols
            || termrows.unwrap_or(120) as usize != self.rows
        {
            debug!(
                "Size updated, from ({}, {}) -> ({:?}, {:?})",
                self.cols, self.rows, termcols, termrows
            );
        }
        self.cols = termcols.unwrap_or(72) as usize;
        self.rows = termrows.unwrap_or(120) as usize;
        if !self.grid.resize(self.cols, self.rows, None) {
            panic!(
                "Terminal size too big: ({} cols, {} rows)",
                self.cols, self.rows
            );
        }
        let _ = self.overlay_grid.resize(self.cols, self.rows, None);

        self.rcv_event(UIEvent::Resize);
        self.display_messages_dirty = true;
        self.display_messages_initialised = false;
        self.display_messages_area = ((0, 0), (0, 0));

        // Invalidate dirty areas.
        self.context.dirty_areas.clear();
    }

    /// Force a redraw for all dirty components.
    pub fn redraw(&mut self) {
        if !self.draw_rate_limit.tick() {
            return;
        }

        for i in 0..self.components.len() {
            self.draw_component(i);
        }
        let mut areas: smallvec::SmallVec<[Area; 8]> =
            self.context.dirty_areas.drain(0..).collect();
        if self.display_messages_active {
            let now = melib::datetime::now();
            if self
                .display_messages_expiration_start
                .map(|t| t + 5 < now)
                .unwrap_or(false)
            {
                self.display_messages_active = false;
                self.display_messages_dirty = true;
                self.display_messages_initialised = false;
                self.display_messages_expiration_start = None;
                areas.push((
                    (0, 0),
                    (self.cols.saturating_sub(1), self.rows.saturating_sub(1)),
                ));
            }
        }

        /* Sort by x_start, ie upper_left corner's x coordinate */
        areas.sort_by(|a, b| (a.0).0.partial_cmp(&(b.0).0).unwrap());

        if self.display_messages_active {
            /* Check if any dirty area intersects with the area occupied by floating notification
             * box */
            let (displ_top, displ_bot) = self.display_messages_area;
            for &((top_x, top_y), (bottom_x, bottom_y)) in &areas {
                self.display_messages_dirty |= !(bottom_y < displ_top.1
                    || displ_bot.1 < top_y
                    || bottom_x < displ_top.0
                    || displ_bot.0 < top_x);
            }
        }
        /* draw each dirty area */
        let rows = self.rows;
        for y in 0..rows {
            let mut segment = None;
            for ((x_start, y_start), (x_end, y_end)) in &areas {
                if y < *y_start || y > *y_end {
                    continue;
                }
                if let Some((x_start, x_end)) = segment.take() {
                    (self.draw_horizontal_segment_fn)(
                        &mut self.grid,
                        self.stdout.as_mut().unwrap(),
                        x_start,
                        x_end,
                        y,
                    );
                }
                match segment {
                    ref mut s @ None => {
                        *s = Some((*x_start, *x_end));
                    }
                    ref mut s @ Some(_) if s.unwrap().1 < *x_start => {
                        (self.draw_horizontal_segment_fn)(
                            &mut self.grid,
                            self.stdout.as_mut().unwrap(),
                            s.unwrap().0,
                            s.unwrap().1,
                            y,
                        );
                        *s = Some((*x_start, *x_end));
                    }
                    ref mut s @ Some(_) if s.unwrap().1 < *x_end => {
                        (self.draw_horizontal_segment_fn)(
                            &mut self.grid,
                            self.stdout.as_mut().unwrap(),
                            s.unwrap().0,
                            s.unwrap().1,
                            y,
                        );
                        *s = Some((s.unwrap().1, *x_end));
                    }
                    Some((_, ref mut x)) => {
                        *x = *x_end;
                    }
                }
            }
            if let Some((x_start, x_end)) = segment {
                (self.draw_horizontal_segment_fn)(
                    &mut self.grid,
                    self.stdout.as_mut().unwrap(),
                    x_start,
                    x_end,
                    y,
                );
            }
        }

        if self.display_messages_dirty && self.display_messages_active {
            if let Some(DisplayMessage {
                ref timestamp,
                ref msg,
                ..
            }) = self.display_messages.get(self.display_messages_pos)
            {
                if !self.display_messages_initialised {
                    {
                        /* Clear area previously occupied by floating notification box */
                        let displ_area = self.display_messages_area;
                        for y in get_y(upper_left!(displ_area))..=get_y(bottom_right!(displ_area)) {
                            (self.draw_horizontal_segment_fn)(
                                &mut self.grid,
                                self.stdout.as_mut().unwrap(),
                                get_x(upper_left!(displ_area)),
                                get_x(bottom_right!(displ_area)),
                                y,
                            );
                        }
                    }
                    let noto_colors = crate::conf::value(&self.context, "status.notification");
                    use crate::melib::text_processing::{Reflow, TextProcessing};

                    let msg_lines = msg.split_lines_reflow(Reflow::All, Some(self.cols / 3));
                    let width = msg_lines
                        .iter()
                        .map(|line| line.grapheme_len() + 4)
                        .max()
                        .unwrap_or(0);

                    let displ_area = place_in_area(
                        (
                            (0, 0),
                            (self.cols.saturating_sub(1), self.rows.saturating_sub(1)),
                        ),
                        (width, std::cmp::min(self.rows, msg_lines.len() + 4)),
                        false,
                        false,
                    );
                    let box_displ_area = create_box(&mut self.overlay_grid, displ_area);
                    for row in self.overlay_grid.bounds_iter(box_displ_area) {
                        for c in row {
                            self.overlay_grid[c]
                                .set_ch(' ')
                                .set_fg(noto_colors.fg)
                                .set_bg(noto_colors.bg)
                                .set_attrs(noto_colors.attrs);
                        }
                    }
                    let ((x, mut y), box_displ_area_bottom_right) = box_displ_area;
                    for line in msg_lines.into_iter().chain(Some(String::new())).chain(Some(
                        melib::datetime::timestamp_to_string(*timestamp, None, false),
                    )) {
                        write_string_to_grid(
                            &line,
                            &mut self.overlay_grid,
                            noto_colors.fg,
                            noto_colors.bg,
                            noto_colors.attrs,
                            ((x, y), box_displ_area_bottom_right),
                            Some(x),
                        );
                        y += 1;
                    }

                    if self.display_messages.len() > 1 {
                        write_string_to_grid(
                            if self.display_messages_pos == 0 {
                                "Next: >"
                            } else if self.display_messages_pos + 1 == self.display_messages.len() {
                                "Prev: <"
                            } else {
                                "Prev: <, Next: >"
                            },
                            &mut self.overlay_grid,
                            noto_colors.fg,
                            noto_colors.bg,
                            noto_colors.attrs,
                            ((x, y), box_displ_area_bottom_right),
                            Some(x),
                        );
                    }
                    self.display_messages_area = displ_area;
                }
                for y in get_y(upper_left!(self.display_messages_area))
                    ..=get_y(bottom_right!(self.display_messages_area))
                {
                    (self.draw_horizontal_segment_fn)(
                        &mut self.overlay_grid,
                        self.stdout.as_mut().unwrap(),
                        get_x(upper_left!(self.display_messages_area)),
                        get_x(bottom_right!(self.display_messages_area)),
                        y,
                    );
                }
            }
            self.display_messages_dirty = false;
        } else if self.display_messages_dirty {
            /* Clear area previously occupied by floating notification box */
            let displ_area = self.display_messages_area;
            for y in get_y(upper_left!(displ_area))..=get_y(bottom_right!(displ_area)) {
                (self.draw_horizontal_segment_fn)(
                    &mut self.grid,
                    self.stdout.as_mut().unwrap(),
                    get_x(upper_left!(displ_area)),
                    get_x(bottom_right!(displ_area)),
                    y,
                );
            }
            self.display_messages_dirty = false;
        }
        if !self.overlay.is_empty() {
            let area = center_area(
                (
                    (0, 0),
                    (self.cols.saturating_sub(1), self.rows.saturating_sub(1)),
                ),
                (
                    if self.cols / 3 > 30 {
                        self.cols / 3
                    } else {
                        self.cols
                    },
                    if self.rows / 5 > 10 {
                        self.rows / 5
                    } else {
                        self.rows
                    },
                ),
            );
            copy_area(&mut self.overlay_grid, &self.grid, area, area);
            self.overlay
                .get_mut(0)
                .unwrap()
                .draw(&mut self.overlay_grid, area, &mut self.context);
            for y in get_y(upper_left!(area))..=get_y(bottom_right!(area)) {
                (self.draw_horizontal_segment_fn)(
                    &mut self.overlay_grid,
                    self.stdout.as_mut().unwrap(),
                    get_x(upper_left!(area)),
                    get_x(bottom_right!(area)),
                    y,
                );
            }
        }
        self.flush();
    }

    /// Draw only a specific `area` on the screen.
    fn draw_horizontal_segment(
        grid: &mut CellBuffer,
        stdout: &mut StateStdout,
        x_start: usize,
        x_end: usize,
        y: usize,
    ) {
        write!(
            stdout,
            "{}",
            cursor::Goto(x_start as u16 + 1, (y + 1) as u16)
        )
        .unwrap();
        let mut current_fg = Color::Default;
        let mut current_bg = Color::Default;
        let mut current_attrs = Attr::DEFAULT;
        write!(stdout, "\x1B[m").unwrap();
        for x in x_start..=x_end {
            let c = &grid[(x, y)];
            if c.attrs() != current_attrs {
                c.attrs().write(current_attrs, stdout).unwrap();
                current_attrs = c.attrs();
            }
            if c.bg() != current_bg {
                c.bg().write_bg(stdout).unwrap();
                current_bg = c.bg();
            }
            if c.fg() != current_fg {
                c.fg().write_fg(stdout).unwrap();
                current_fg = c.fg();
            }
            if !c.empty() {
                write!(stdout, "{}", c.ch()).unwrap();
            }
        }
    }

    fn draw_horizontal_segment_no_color(
        grid: &mut CellBuffer,
        stdout: &mut StateStdout,
        x_start: usize,
        x_end: usize,
        y: usize,
    ) {
        write!(
            stdout,
            "{}",
            cursor::Goto(x_start as u16 + 1, (y + 1) as u16)
        )
        .unwrap();
        let mut current_attrs = Attr::DEFAULT;
        write!(stdout, "\x1B[m").unwrap();
        for x in x_start..=x_end {
            let c = &grid[(x, y)];
            if c.attrs() != current_attrs {
                c.attrs().write(current_attrs, stdout).unwrap();
                current_attrs = c.attrs();
            }
            if !c.empty() {
                write!(stdout, "{}", c.ch()).unwrap();
            }
        }
    }

    /// Draw the entire screen from scratch.
    pub fn render(&mut self) {
        self.update_size();
        let cols = self.cols;
        let rows = self.rows;
        self.context
            .dirty_areas
            .push_back(((0, 0), (cols - 1, rows - 1)));

        self.redraw();
    }

    pub fn draw_component(&mut self, idx: usize) {
        let component = &mut self.components[idx];
        let upper_left = (0, 0);
        let bottom_right = (self.cols - 1, self.rows - 1);

        if component.is_dirty() {
            component.draw(
                &mut self.grid,
                (upper_left, bottom_right),
                &mut self.context,
            );
        }
    }

    pub fn can_quit_cleanly(&mut self) -> bool {
        let State {
            ref mut components,
            ref context,
            ..
        } = self;
        components.iter_mut().all(|c| c.can_quit_cleanly(context))
    }

    pub fn register_component(&mut self, component: Box<dyn Component>) {
        self.components.push(component);
    }

    /// Convert user commands to actions/method calls.
    fn exec_command(&mut self, cmd: Action) {
        match cmd {
            SetEnv(key, val) => {
                env::set_var(key.as_str(), val.as_str());
            }
            PrintEnv(key) => {
                self.context
                    .replies
                    .push_back(UIEvent::StatusEvent(StatusEvent::DisplayMessage(
                        env::var(key.as_str()).unwrap_or_else(|e| e.to_string()),
                    )));
            }
            Mailbox(account_name, op) => {
                if let Some(account) = self
                    .context
                    .accounts
                    .values_mut()
                    .find(|a| a.name() == account_name)
                {
                    if let Err(err) = account.mailbox_operation(op) {
                        self.context.replies.push_back(UIEvent::StatusEvent(
                            StatusEvent::DisplayMessage(err.to_string()),
                        ));
                    }
                } else {
                    self.context.replies.push_back(UIEvent::StatusEvent(
                        StatusEvent::DisplayMessage(format!(
                            "Account with name `{}` not found.",
                            account_name
                        )),
                    ));
                }
            }
            #[cfg(feature = "sqlite3")]
            AccountAction(ref account_name, ReIndex) => {
                let account_index = if let Some(a) = self
                    .context
                    .accounts
                    .iter()
                    .position(|(_, acc)| acc.name() == account_name)
                {
                    a
                } else {
                    self.context.replies.push_back(UIEvent::Notification(
                        None,
                        format!("Account {} was not found.", account_name),
                        Some(NotificationType::Error(ErrorKind::None)),
                    ));
                    return;
                };
                if *self.context.accounts[account_index]
                    .settings
                    .conf
                    .search_backend()
                    != crate::conf::SearchBackend::Sqlite3
                {
                    self.context.replies.push_back(UIEvent::Notification(
                        None,
                        format!(
                            "Account {} doesn't have an sqlite3 search backend.",
                            account_name
                        ),
                        Some(NotificationType::Error(ErrorKind::None)),
                    ));
                    return;
                }
                match crate::sqlite3::index(&mut self.context, account_index) {
                    Ok(job) => {
                        let handle = self.context.job_executor.spawn_blocking(job);
                        self.context.accounts[account_index].active_jobs.insert(
                            handle.job_id,
                            crate::conf::accounts::JobRequest::Generic {
                                name: "Message index rebuild".into(),
                                handle,
                                on_finish: None,
                                logging_level: melib::LoggingLevel::INFO,
                            },
                        );
                        self.context.replies.push_back(UIEvent::Notification(
                            None,
                            "Message index rebuild started.".to_string(),
                            Some(NotificationType::Info),
                        ));
                    }
                    Err(err) => {
                        self.context.replies.push_back(UIEvent::Notification(
                            Some("Message index rebuild failed".to_string()),
                            err.to_string(),
                            Some(NotificationType::Error(err.kind)),
                        ));
                    }
                }
            }
            #[cfg(not(feature = "sqlite3"))]
            AccountAction(ref account_name, ReIndex) => {
                self.context.replies.push_back(UIEvent::Notification(
                    None,
                    "Message index rebuild failed: meli is not built with sqlite3 support."
                        .to_string(),
                    Some(NotificationType::Error(ErrorKind::None)),
                ));
            }
            AccountAction(ref account_name, PrintAccountSetting(ref setting)) => {
                let path = setting.split(".").collect::<SmallVec<[&str; 16]>>();
                if let Some(pos) = self
                    .context
                    .accounts
                    .iter()
                    .position(|(_h, a)| a.name() == account_name)
                {
                    self.context.replies.push_back(UIEvent::StatusEvent(
                        StatusEvent::UpdateStatus(format!(
                            "{}",
                            self.context.accounts[pos]
                                .settings
                                .lookup("settings", &path)
                                .unwrap_or_else(|err| err.to_string())
                        )),
                    ));
                } else {
                    self.context.replies.push_back(UIEvent::Notification(
                        None,
                        format!("Account {} was not found.", account_name),
                        Some(NotificationType::Error(ErrorKind::None)),
                    ));
                    return;
                }
            }
            PrintSetting(ref setting) => {
                let path = setting.split(".").collect::<SmallVec<[&str; 16]>>();
                self.context
                    .replies
                    .push_back(UIEvent::StatusEvent(StatusEvent::UpdateStatus(format!(
                        "{}",
                        self.context
                            .settings
                            .lookup("settings", &path)
                            .unwrap_or_else(|err| err.to_string())
                    ))));
            }
            ToggleMouse => {
                self.mouse = !self.mouse;
                self.set_mouse(self.mouse);
                self.rcv_event(UIEvent::StatusEvent(StatusEvent::SetMouse(self.mouse)));
            }
            Quit => {
                self.context
                    .sender
                    .send(ThreadEvent::Input((
                        self.context.settings.shortcuts.general.quit.clone(),
                        vec![],
                    )))
                    .unwrap();
            }
            v => {
                self.rcv_event(UIEvent::Action(v));
            }
        }
    }

    /// The application's main loop sends `UIEvents` to state via this method.
    pub fn rcv_event(&mut self, mut event: UIEvent) {
        if let UIEvent::Input(_) = event {
            if self.display_messages_expiration_start.is_none() {
                self.display_messages_expiration_start = Some(melib::datetime::now());
            }
        }

        match event {
            // Command type is handled only by State.
            UIEvent::Command(cmd) => {
                if let Ok(action) = parse_command(&cmd.as_bytes()) {
                    if action.needs_confirmation() {
                        self.overlay.push(Box::new(UIConfirmationDialog::new(
                            "You sure?",
                            vec![(true, "yes".to_string()), (false, "no".to_string())],
                            true,
                            Some(Box::new(move |id: ComponentId, result: bool| {
                                Some(UIEvent::FinishedUIDialog(
                                    id,
                                    Box::new(if result { Some(action) } else { None }),
                                ))
                            })),
                            &mut self.context,
                        )));
                    } else if let Action::ReloadConfiguration = action {
                        match Settings::new().and_then(|new_settings| {
                            let old_accounts = self.context.settings.accounts.keys().collect::<std::collections::HashSet<&String>>();
                            let new_accounts = new_settings.accounts.keys().collect::<std::collections::HashSet<&String>>();
                            if old_accounts != new_accounts {
                                return Err("cannot reload account configuration changes; restart meli instead.".into());
                            }
                            for (key, acc) in new_settings.accounts.iter() {
                                if toml::Value::try_from(&acc) != toml::Value::try_from(&self.context.settings.accounts[key]) {
                                    return Err("cannot reload account configuration changes; restart meli instead.".into());
                                }
                            }
                            if toml::Value::try_from(&new_settings) == toml::Value::try_from(&self.context.settings) {
                                return Err("No changes detected.".into());
                            }
                            Ok(new_settings)
                        }) {
                            Ok(new_settings) => {
                                let old_settings = std::mem::replace(&mut self.context.settings, new_settings);
                                self.context.replies.push_back(UIEvent::ConfigReload {
                                    old_settings
                                });
                                self.context.replies.push_back(UIEvent::Resize);
                            }
                            Err(err) => {
                                self.context.replies.push_back(UIEvent::StatusEvent(
                                        StatusEvent::DisplayMessage(format!(
                                                "Could not load configuration: {}",
                                                err
                                        )),
                                ));
                            }
                        }
                    } else {
                        self.exec_command(action);
                    }
                } else {
                    self.context.replies.push_back(UIEvent::StatusEvent(
                        StatusEvent::DisplayMessage("invalid command".to_string()),
                    ));
                }
                return;
            }
            UIEvent::Fork(ForkType::Finished) => {
                /*
                 * Fork has finished in the past.
                 * We're back in the AlternateScreen, but the cursor is reset to Shown, so fix
                 * it.
                write!(self.stdout(), "{}", cursor::Hide,).unwrap();
                self.flush();
                 */
                self.switch_to_main_screen();
                self.switch_to_alternate_screen();
                self.context.restore_input();
                return;
            }
            UIEvent::Fork(ForkType::Generic(child)) => {
                self.context.children.push(child);
                return;
            }
            UIEvent::Fork(child) => {
                self.mode = UIMode::Fork;
                self.child = Some(child);
                return;
            }
            UIEvent::BackendEvent(
                account_hash,
                BackendEvent::Notice {
                    ref description,
                    ref content,
                    level,
                },
            ) => {
                log(
                    format!(
                        "{}: {}{}{}",
                        self.context.accounts[&account_hash].name(),
                        description.as_ref().map(|s| s.as_str()).unwrap_or(""),
                        if description.is_some() { ": " } else { "" },
                        content.as_str()
                    ),
                    level,
                );
                self.rcv_event(UIEvent::StatusEvent(StatusEvent::DisplayMessage(
                    content.to_string(),
                )));
                return;
            }
            UIEvent::BackendEvent(_, BackendEvent::Refresh(refresh_event)) => {
                self.refresh_event(refresh_event);
                return;
            }
            UIEvent::ChangeMode(m) => {
                self.context
                    .sender
                    .send(ThreadEvent::UIEvent(UIEvent::ChangeMode(m)))
                    .unwrap();
            }
            UIEvent::Timer(id) if id == self.draw_rate_limit.id() => {
                self.draw_rate_limit.reset();
                self.redraw();
                return;
            }
            UIEvent::Input(Key::Alt('<')) => {
                self.display_messages_expiration_start = Some(melib::datetime::now());
                self.display_messages_active = true;
                self.display_messages_initialised = false;
                self.display_messages_dirty = true;
                self.display_messages_pos = self.display_messages_pos.saturating_sub(1);
                return;
            }
            UIEvent::Input(Key::Alt('>')) => {
                self.display_messages_expiration_start = Some(melib::datetime::now());
                self.display_messages_active = true;
                self.display_messages_initialised = false;
                self.display_messages_dirty = true;
                self.display_messages_pos = std::cmp::min(
                    self.display_messages.len().saturating_sub(1),
                    self.display_messages_pos + 1,
                );
                return;
            }
            UIEvent::StatusEvent(StatusEvent::DisplayMessage(ref msg)) => {
                self.display_messages.push(DisplayMessage {
                    timestamp: melib::datetime::now(),
                    msg: msg.clone(),
                });
                self.display_messages_active = true;
                self.display_messages_initialised = false;
                self.display_messages_dirty = true;
                self.display_messages_expiration_start = None;
                self.display_messages_pos = self.display_messages.len() - 1;
                self.redraw();
            }
            UIEvent::ComponentKill(ref id) if self.overlay.iter().any(|c| c.id() == *id) => {
                let pos = self.overlay.iter().position(|c| c.id() == *id).unwrap();
                self.overlay.remove(pos);
            }
            UIEvent::FinishedUIDialog(ref id, ref mut results)
                if self.overlay.iter().any(|c| c.id() == *id) =>
            {
                if let Some(ref mut action @ Some(_)) = results.downcast_mut::<Option<Action>>() {
                    self.exec_command(action.take().unwrap());

                    return;
                }
            }
            UIEvent::Callback(callback_fn) => {
                (callback_fn.0)(&mut self.context);
                return;
            }
            UIEvent::GlobalUIDialog(dialog) => {
                self.overlay.push(dialog);
                return;
            }
            _ => {}
        }
        let Self {
            ref mut components,
            ref mut context,
            ref mut overlay,
            ..
        } = self;

        /* inform each component */
        for c in overlay.iter_mut().chain(components.iter_mut()) {
            if c.process_event(&mut event, context) {
                break;
            }
        }

        if !self.context.replies.is_empty() {
            let replies: smallvec::SmallVec<[UIEvent; 8]> =
                self.context.replies.drain(0..).collect();
            // Pass replies to self and call count on the map iterator to force evaluation
            replies.into_iter().map(|r| self.rcv_event(r)).count();
        }
    }

    pub fn try_wait_on_child(&mut self) -> Option<bool> {
        let should_return_flag = match self.child {
            Some(ForkType::NewDraft(_, ref mut c)) => {
                let w = c.try_wait();
                match w {
                    Ok(Some(_)) => true,
                    Ok(None) => false,
                    Err(e) => {
                        log(
                            format!("Failed to wait on editor process: {}", e.to_string()),
                            ERROR,
                        );
                        return None;
                    }
                }
            }
            Some(ForkType::Generic(ref mut c)) => {
                let w = c.try_wait();
                match w {
                    Ok(Some(_)) => true,
                    Ok(None) => false,
                    Err(e) => {
                        log(
                            format!("Failed to wait on child process: {}", e.to_string()),
                            ERROR,
                        );
                        return None;
                    }
                }
            }
            Some(ForkType::Finished) => {
                /* Fork has already finished */
                self.child = None;
                return None;
            }
            _ => {
                return None;
            }
        };
        if should_return_flag {
            return Some(true);
        }
        Some(false)
    }
    fn flush(&mut self) {
        if let Some(s) = self.stdout.as_mut() {
            s.flush().unwrap();
        }
    }
    fn stdout(&mut self) -> &mut StateStdout {
        self.stdout.as_mut().unwrap()
    }

    pub fn check_accounts(&mut self) {
        let mut ctr = 0;
        for i in 0..self.context.accounts.len() {
            if self.context.is_online_idx(i).is_ok() {
                ctr += 1;
            }
        }
        if ctr != self.context.accounts.len() {
            self.timer.thread().unpark();
        }
        self.context.input_thread.check();
    }
}

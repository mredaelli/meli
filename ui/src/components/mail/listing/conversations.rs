/*
 * meli - ui crate.
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

use super::*;
use crate::components::utilities::PageMovement;
use std::iter::FromIterator;
use std::ops::{Deref, DerefMut};

#[derive(Debug)]
pub(super) struct EntryStrings {
    pub(super) date: DateString,
    pub(super) subject: SubjectString,
    pub(super) flag: FlagString,
    pub(super) from: FromString,
    pub(super) tags: TagString,
}

macro_rules! address_list {
    (($name:expr) as comma_sep_list) => {{
        let mut ret: String =
            $name
                .into_iter()
                .fold(String::new(), |mut s: String, n: &Address| {
                    s.extend(n.to_string().chars());
                    s.push_str(", ");
                    s
                });
        ret.pop();
        ret.pop();
        ret
    }};
}

macro_rules! column_str {
    (
        struct $name:ident($($t:ty),+)) => {
        #[derive(Debug)]
        pub(super) struct $name($(pub $t),+);

        impl Deref for $name {
            type Target = String;
            fn deref(&self) -> &String {
                &self.0
            }
        }
        impl DerefMut for $name {
            fn deref_mut(&mut self) -> &mut String {
                &mut self.0
            }
        }
    };
}

column_str!(struct DateString(String));
column_str!(struct FromString(String));
column_str!(struct SubjectString(String));
column_str!(struct FlagString(String));
column_str!(struct TagString(String, StackVec<Color>));

/// A list of all mail (`Envelope`s) in a `Mailbox`. On `\n` it opens the `Envelope` content in a
/// `ThreadView`.
#[derive(Debug)]
pub struct ConversationsListing {
    /// (x, y, z): x is accounts, y is folders, z is index inside a folder.
    cursor_pos: (usize, usize, usize),
    new_cursor_pos: (usize, usize, usize),
    length: usize,
    sort: (SortField, SortOrder),
    subsort: (SortField, SortOrder),
    all_threads: fnv::FnvHashSet<ThreadHash>,
    order: FnvHashMap<ThreadHash, usize>,
    /// Cache current view.
    content: CellBuffer,

    filter_term: String,
    filtered_selection: Vec<ThreadHash>,
    filtered_order: FnvHashMap<ThreadHash, usize>,
    selection: FnvHashMap<ThreadHash, bool>,
    /// If we must redraw on next redraw event
    dirty: bool,
    force_draw: bool,
    /// If `self.view` exists or not.
    unfocused: bool,
    view: ThreadView,
    row_updates: StackVec<ThreadHash>,

    movement: Option<PageMovement>,
    id: ComponentId,
}

impl MailListingTrait for ConversationsListing {
    fn row_updates(&mut self) -> &mut StackVec<ThreadHash> {
        &mut self.row_updates
    }

    fn get_focused_items(&self, context: &Context) -> StackVec<ThreadHash> {
        let is_selection_empty = self.selection.values().cloned().any(std::convert::identity);
        let i = [self.get_thread_under_cursor(self.cursor_pos.2, context)];
        let cursor_iter;
        let sel_iter = if is_selection_empty {
            cursor_iter = None;
            Some(self.selection.iter().filter(|(_, v)| **v).map(|(k, _)| k))
        } else {
            cursor_iter = Some(i.iter());
            None
        };
        let iter = sel_iter
            .into_iter()
            .flatten()
            .chain(cursor_iter.into_iter().flatten())
            .cloned();
        StackVec::from_iter(iter.into_iter())
    }
}

impl ListingTrait for ConversationsListing {
    fn coordinates(&self) -> (usize, usize) {
        (self.new_cursor_pos.0, self.new_cursor_pos.1)
    }

    fn set_coordinates(&mut self, coordinates: (usize, usize)) {
        self.new_cursor_pos = (coordinates.0, coordinates.1, 0);
        self.unfocused = false;
        self.filtered_selection.clear();
        self.filtered_order.clear();
        self.filter_term.clear();
        self.row_updates.clear();
    }

    fn highlight_line(&mut self, grid: &mut CellBuffer, area: Area, idx: usize, context: &Context) {
        if self.length == 0 {
            return;
        }
        let i = self.get_thread_under_cursor(idx, context);

        let account = &context.accounts[self.cursor_pos.0];
        let folder_hash = account[self.cursor_pos.1].unwrap().folder.hash();
        let threads = &account.collection.threads[&folder_hash];
        let thread_node = &threads.thread_nodes[&i];

        let fg_color = if thread_node.has_unseen() {
            Color::Byte(0)
        } else {
            Color::Default
        };
        let bg_color = if self.cursor_pos.2 == idx {
            Color::Byte(246)
        } else if self.selection[&i] {
            Color::Byte(210)
        } else if thread_node.has_unseen() {
            Color::Byte(251)
        } else {
            Color::Default
        };

        copy_area(
            grid,
            &self.content,
            area,
            ((0, 3 * idx), pos_dec(self.content.size(), (1, 1))),
        );

        let padding_fg = if context.settings.terminal.theme == "light" {
            Color::Byte(254)
        } else {
            Color::Byte(235)
        };

        let (upper_left, bottom_right) = area;
        let width = self.content.size().0;
        let (x, y) = upper_left;
        if self.cursor_pos.2 == idx || self.selection[&i] {
            for x in x..=get_x(bottom_right) {
                grid[(x, y)].set_fg(fg_color);
                grid[(x, y)].set_bg(bg_color);

                grid[(x, y + 1)].set_fg(fg_color);
                grid[(x, y + 1)].set_bg(bg_color);

                grid[(x, y + 2)].set_fg(padding_fg);
                grid[(x, y + 2)].set_bg(bg_color);
            }
        } else if width < width!(area) {
            /* fill any remaining columns, if our view is wider than self.content */
            for x in (x + width)..=get_x(bottom_right) {
                grid[(x, y)].set_fg(fg_color);
                grid[(x, y)].set_bg(bg_color);

                grid[(x, y + 1)].set_fg(fg_color);
                grid[(x, y + 1)].set_bg(bg_color);

                grid[(x, y + 2)].set_fg(padding_fg);
                grid[(x, y + 2)].set_bg(bg_color);
            }
        }
        return;
    }
    /// Draw the list of `Envelope`s.
    fn draw_list(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if self.cursor_pos.1 != self.new_cursor_pos.1 || self.cursor_pos.0 != self.new_cursor_pos.0
        {
            self.refresh_mailbox(context);
        }
        let upper_left = upper_left!(area);
        let bottom_right = bottom_right!(area);
        if self.length == 0 {
            clear_area(grid, area);
            copy_area(
                grid,
                &self.content,
                area,
                ((0, 0), pos_dec(self.content.size(), (1, 1))),
            );
            context.dirty_areas.push_back(area);
            return;
        }
        let rows = (get_y(bottom_right) - get_y(upper_left) + 1) / 3;
        let pad = (get_y(bottom_right) - get_y(upper_left) + 1) % 3;

        if let Some(mvm) = self.movement.take() {
            match mvm {
                PageMovement::Up(amount) => {
                    self.new_cursor_pos.2 = self.new_cursor_pos.2.saturating_sub(amount);
                }
                PageMovement::PageUp(multiplier) => {
                    self.new_cursor_pos.2 = self.new_cursor_pos.2.saturating_sub(rows * multiplier);
                }
                PageMovement::Down(amount) => {
                    if self.new_cursor_pos.2 + amount + 1 < self.length {
                        self.new_cursor_pos.2 += amount;
                    } else {
                        self.new_cursor_pos.2 = self.length - 1;
                    }
                }
                PageMovement::PageDown(multiplier) => {
                    if self.new_cursor_pos.2 + rows * multiplier + 1 < self.length {
                        self.new_cursor_pos.2 += rows * multiplier;
                    } else if self.new_cursor_pos.2 + rows * multiplier > self.length {
                        self.new_cursor_pos.2 = self.length - 1;
                    } else {
                        self.new_cursor_pos.2 = (self.length / rows) * rows;
                    }
                }
                PageMovement::Right(_) | PageMovement::Left(_) => {}
                PageMovement::Home => {
                    self.new_cursor_pos.2 = 0;
                }
                PageMovement::End => {
                    self.new_cursor_pos.2 = self.length - 1;
                }
            }
        }

        let prev_page_no = (self.cursor_pos.2).wrapping_div(rows);
        let page_no = (self.new_cursor_pos.2).wrapping_div(rows);

        let top_idx = page_no * rows;

        /* If cursor position has changed, remove the highlight from the previous position and
         * apply it in the new one. */
        if self.cursor_pos.2 != self.new_cursor_pos.2 && prev_page_no == page_no {
            let old_cursor_pos = self.cursor_pos;
            self.cursor_pos = self.new_cursor_pos;
            for idx in &[old_cursor_pos.2, self.new_cursor_pos.2] {
                if *idx >= self.length {
                    continue; //bounds check
                }
                let new_area = (
                    set_y(upper_left, get_y(upper_left) + 3 * (*idx % rows)),
                    set_y(bottom_right, get_y(upper_left) + 3 * (*idx % rows) + 2),
                );
                self.highlight_line(grid, new_area, *idx, context);
                context.dirty_areas.push_back(new_area);
            }
            return;
        } else if self.cursor_pos != self.new_cursor_pos {
            self.cursor_pos = self.new_cursor_pos;
        }
        if self.new_cursor_pos.2 >= self.length {
            self.new_cursor_pos.2 = self.length - 1;
            self.cursor_pos.2 = self.new_cursor_pos.2;
        }

        clear_area(grid, area);
        /* Page_no has changed, so draw new page */
        copy_area(
            grid,
            &self.content,
            (
                upper_left,
                set_x(
                    bottom_right,
                    std::cmp::min(
                        get_x(bottom_right),
                        get_x(upper_left) + self.content.size().0,
                    ),
                ),
            ),
            ((0, 3 * top_idx), pos_dec(self.content.size(), (1, 1))),
        );

        /* TODO: highlight selected entries */
        self.highlight_line(
            grid,
            (
                pos_inc(upper_left, (0, 3 * (self.cursor_pos.2 % rows))),
                set_y(
                    bottom_right,
                    get_y(upper_left) + 3 * (self.cursor_pos.2 % rows) + 2,
                ),
            ),
            self.cursor_pos.2,
            context,
        );

        /* calculate how many entries are visible in this page */
        let (pad, rows) = if top_idx + rows > self.length {
            clear_area(
                grid,
                (
                    pos_inc(upper_left, (0, 3 * (self.length - top_idx))),
                    bottom_right,
                ),
            );
            (0, self.length - top_idx)
        } else {
            (pad, rows)
        };

        /* fill any remaining columns, if our view is wider than self.content */
        let width = self.content.size().0;
        let padding_fg = if context.settings.terminal.theme == "light" {
            Color::Byte(254)
        } else {
            Color::Byte(235)
        };

        if width < width!(area) {
            let y_offset = get_y(upper_left);
            for y in 0..rows {
                let bg_color = grid[(get_x(upper_left) + width - 1, y_offset + 3 * y)].bg();
                for x in (get_x(upper_left) + width)..=get_x(bottom_right) {
                    grid[(x, y_offset + 3 * y)].set_bg(bg_color);
                    grid[(x, y_offset + 3 * y + 1)].set_ch('▁');
                    grid[(x, y_offset + 3 * y + 2)].set_fg(Color::Default);
                    grid[(x, y_offset + 3 * y + 1)].set_bg(bg_color);
                    grid[(x, y_offset + 3 * y + 2)].set_ch('▓');
                    grid[(x, y_offset + 3 * y + 2)].set_fg(padding_fg);
                    grid[(x, y_offset + 3 * y + 2)].set_bg(bg_color);
                }
            }
            if pad > 0 {
                let y = 3 * rows;
                let bg_color = grid[(get_x(upper_left) + width - 1, y_offset + y)].bg();
                for x in (get_x(upper_left) + width)..=get_x(bottom_right) {
                    grid[(x, y_offset + y)].set_bg(bg_color);
                    grid[(x, y_offset + y + 1)].set_ch('▁');
                    grid[(x, y_offset + y + 1)].set_bg(bg_color);
                    if pad == 2 {
                        grid[(x, y_offset + y + 2)].set_fg(Color::Default);
                        grid[(x, y_offset + y + 2)].set_ch('▓');
                        grid[(x, y_offset + y + 2)].set_fg(padding_fg);
                        grid[(x, y_offset + y + 2)].set_bg(bg_color);
                    }
                }
            }
        }

        context.dirty_areas.push_back(area);
    }

    fn filter(&mut self, filter_term: &str, context: &Context) {
        if filter_term.is_empty() {
            return;
        }

        self.order.clear();
        self.selection.clear();
        self.length = 0;
        self.filtered_selection.clear();
        self.filtered_order.clear();
        self.filter_term = filter_term.to_string();
        self.row_updates.clear();
        for v in self.selection.values_mut() {
            *v = false;
        }

        let account = &context.accounts[self.cursor_pos.0];
        let folder_hash = account[self.cursor_pos.1].unwrap().folder.hash();
        match account.search(&self.filter_term, self.sort, folder_hash) {
            Ok(results) => {
                let threads = &account.collection.threads[&folder_hash];
                for env_hash in results {
                    if !account.collection.contains_key(&env_hash) {
                        continue;
                    }
                    let env_hash_thread_hash = account.collection.get_env(env_hash).thread();
                    if !threads.thread_nodes.contains_key(&env_hash_thread_hash) {
                        continue;
                    }
                    let thread_group =
                        melib::find_root_hash(&threads.thread_nodes, env_hash_thread_hash);
                    if self.filtered_order.contains_key(&thread_group) {
                        continue;
                    }
                    if self.all_threads.contains(&thread_group) {
                        self.filtered_selection.push(thread_group);
                        self.filtered_order
                            .insert(thread_group, self.filtered_selection.len() - 1);
                    }
                }
                if !self.filtered_selection.is_empty() {
                    threads.vec_inner_sort_by(
                        &mut self.filtered_selection,
                        self.sort,
                        &context.accounts[self.cursor_pos.0].collection.envelopes,
                    );
                    self.new_cursor_pos.2 =
                        std::cmp::min(self.filtered_selection.len() - 1, self.cursor_pos.2);
                } else {
                    self.content =
                        CellBuffer::new_with_context(0, 0, Cell::with_char(' '), context);
                }
                self.redraw_list(context);
            }
            Err(e) => {
                self.cursor_pos.2 = 0;
                self.new_cursor_pos.2 = 0;
                let message = format!(
                    "Encountered an error while searching for `{}`: {}.",
                    self.filter_term, e
                );
                log(
                    format!("Failed to search for term {}: {}", self.filter_term, e),
                    ERROR,
                );
                self.content =
                    CellBuffer::new_with_context(message.len(), 1, Cell::with_char(' '), context);
                write_string_to_grid(
                    &message,
                    &mut self.content,
                    Color::Default,
                    Color::Default,
                    Attr::Default,
                    ((0, 0), (message.len() - 1, 0)),
                    None,
                );
            }
        }
    }

    fn set_movement(&mut self, mvm: PageMovement) {
        self.movement = Some(mvm);
        self.set_dirty(true);
    }
}

impl fmt::Display for ConversationsListing {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "mail")
    }
}

impl Default for ConversationsListing {
    fn default() -> Self {
        ConversationsListing::new()
    }
}

impl ConversationsListing {
    const DESCRIPTION: &'static str = "compact listing";
    fn new() -> Self {
        ConversationsListing {
            cursor_pos: (0, 1, 0),
            new_cursor_pos: (0, 0, 0),
            length: 0,
            sort: (Default::default(), Default::default()),
            subsort: (SortField::Date, SortOrder::Desc),
            order: FnvHashMap::default(),
            all_threads: fnv::FnvHashSet::default(),
            filter_term: String::new(),
            filtered_selection: Vec::new(),
            filtered_order: FnvHashMap::default(),
            selection: FnvHashMap::default(),
            row_updates: StackVec::new(),
            content: Default::default(),
            dirty: true,
            force_draw: true,
            unfocused: false,
            view: ThreadView::default(),

            movement: None,
            id: ComponentId::new_v4(),
        }
    }
    pub(super) fn make_entry_string(
        &self,
        e: &Envelope,
        context: &Context,
        from: &Vec<Address>,
        thread_node: &ThreadNode,
        is_snoozed: bool,
    ) -> EntryStrings {
        let folder_hash = &context.accounts[self.cursor_pos.0][self.cursor_pos.1]
            .unwrap()
            .folder
            .hash();
        let folder = &context.accounts[self.cursor_pos.0].folder_confs[&folder_hash];
        let mut tags = String::new();
        let mut colors = StackVec::new();
        let backend_lck = context.accounts[self.cursor_pos.0].backend.read().unwrap();
        if let Some(t) = backend_lck.tags() {
            let tags_lck = t.read().unwrap();
            for t in e.labels().iter() {
                if folder
                    .conf_override
                    .tags
                    .as_ref()
                    .map(|s| s.ignore_tags.contains(t))
                    .unwrap_or(false)
                {
                    continue;
                }
                tags.push(' ');
                tags.push_str(tags_lck.get(t).as_ref().unwrap());
                tags.push(' ');
                if let Some(&c) = folder
                    .conf_override
                    .tags
                    .as_ref()
                    .map(|s| s.colors.get(t))
                    .unwrap_or(None)
                {
                    colors.push(c);
                } else {
                    colors.push(Color::Byte(8));
                }
            }
            if !tags.is_empty() {
                tags.pop();
            }
        }
        let mut subject = e.subject().to_string();
        subject.truncate_at_boundary(150);
        if thread_node.len() > 0 {
            EntryStrings {
                date: DateString(ConversationsListing::format_date(thread_node)),
                subject: SubjectString(format!("{} ({})", subject, thread_node.len(),)),
                flag: FlagString(format!(
                    "{}{}",
                    if e.has_attachments() { "📎" } else { "" },
                    if is_snoozed { "💤" } else { "" }
                )),
                from: FromString(address_list!((from) as comma_sep_list)),
                tags: TagString(tags, colors),
            }
        } else {
            EntryStrings {
                date: DateString(ConversationsListing::format_date(thread_node)),
                subject: SubjectString(subject),
                flag: FlagString(format!(
                    "{}{}",
                    if e.has_attachments() { "📎" } else { "" },
                    if is_snoozed { "💤" } else { "" }
                )),
                from: FromString(address_list!((from) as comma_sep_list)),
                tags: TagString(tags, colors),
            }
        }
    }

    /// Fill the `self.data_columns` `CellBuffers` with the contents of the account folder the user has
    /// chosen.
    fn refresh_mailbox(&mut self, context: &mut Context) {
        self.dirty = true;
        let old_cursor_pos = self.cursor_pos;
        if !(self.cursor_pos.0 == self.new_cursor_pos.0
            && self.cursor_pos.1 == self.new_cursor_pos.1)
        {
            self.cursor_pos.2 = 0;
            self.new_cursor_pos.2 = 0;
        }
        self.cursor_pos.1 = self.new_cursor_pos.1;
        self.cursor_pos.0 = self.new_cursor_pos.0;
        let folder_hash = if let Some(h) = context.accounts[self.cursor_pos.0]
            .folders_order
            .get(self.cursor_pos.1)
        {
            *h
        } else {
            self.cursor_pos.1 = old_cursor_pos.1;
            self.dirty = false;
            return;
        };

        // Get mailbox as a reference.
        //
        match context.accounts[self.cursor_pos.0].status(folder_hash) {
            Ok(()) => {}
            Err(_) => {
                let message: String = context.accounts[self.cursor_pos.0][folder_hash].to_string();
                self.content =
                    CellBuffer::new_with_context(message.len(), 1, Cell::with_char(' '), context);
                self.length = 0;
                write_string_to_grid(
                    message.as_str(),
                    &mut self.content,
                    Color::Default,
                    Color::Default,
                    Attr::Default,
                    ((0, 0), (message.len() - 1, 0)),
                    None,
                );
                return;
            }
        }
        if old_cursor_pos == self.new_cursor_pos {
            self.view.update(context);
        } else if self.unfocused {
            self.view = ThreadView::new(self.new_cursor_pos, None, context);
        }

        self.redraw_list(context);
    }

    fn redraw_list(&mut self, context: &Context) {
        let account = &context.accounts[self.cursor_pos.0];
        let mailbox = &account[self.cursor_pos.1].unwrap();

        let threads = &account.collection.threads[&mailbox.folder.hash()];
        self.order.clear();
        self.selection.clear();
        self.length = 0;
        let mut rows = Vec::with_capacity(1024);
        let mut max_entry_columns = 0;

        threads.sort_by(self.sort, self.subsort, &account.collection.envelopes);

        let mut refresh_mailbox = false;
        let threads_iter = if self.filter_term.is_empty() {
            refresh_mailbox = true;
            self.all_threads.clear();
            Box::new(threads.root_iter()) as Box<dyn Iterator<Item = ThreadHash>>
        } else {
            Box::new(self.filtered_selection.iter().map(|h| *h))
                as Box<dyn Iterator<Item = ThreadHash>>
        };

        let mut from_address_list = Vec::new();
        let mut from_address_set: std::collections::HashSet<Vec<u8>> =
            std::collections::HashSet::new();
        for (idx, root_idx) in threads_iter.enumerate() {
            self.length += 1;
            let thread_node = &threads.thread_nodes()[&root_idx];
            let i = thread_node.message().unwrap_or_else(|| {
                let mut iter_ptr = thread_node.children()[0];
                while threads.thread_nodes()[&iter_ptr].message().is_none() {
                    iter_ptr = threads.thread_nodes()[&iter_ptr].children()[0];
                }
                threads.thread_nodes()[&iter_ptr].message().unwrap()
            });
            if !context.accounts[self.cursor_pos.0].contains_key(i) {
                debug!("key = {}", i);
                debug!(
                    "name = {} {}",
                    mailbox.name(),
                    context.accounts[self.cursor_pos.0].name()
                );
                debug!("{:#?}", context.accounts);

                panic!();
            }
            from_address_list.clear();
            from_address_set.clear();
            let mut stack = StackVec::new();
            stack.push(root_idx);
            while let Some(h) = stack.pop() {
                let env_hash = if let Some(h) = threads.thread_nodes()[&h].message() {
                    h
                } else {
                    break;
                };

                let envelope: &EnvelopeRef = &context.accounts[self.cursor_pos.0]
                    .collection
                    .get_env(env_hash);
                for addr in envelope.from().iter() {
                    if from_address_set.contains(addr.raw()) {
                        continue;
                    }
                    from_address_set.insert(addr.raw().to_vec());
                    from_address_list.push(addr.clone());
                }
                for c in threads.thread_nodes()[&h].children() {
                    stack.push(*c);
                }
            }

            let root_envelope: &EnvelopeRef =
                &context.accounts[self.cursor_pos.0].collection.get_env(i);

            let strings = self.make_entry_string(
                root_envelope,
                context,
                &from_address_list,
                thread_node,
                threads.is_snoozed(root_idx),
            );
            max_entry_columns = std::cmp::max(
                max_entry_columns,
                strings.flag.len()
                    + 3
                    + strings.subject.grapheme_width()
                    + 1
                    + strings.tags.grapheme_width(),
            );
            max_entry_columns = std::cmp::max(
                max_entry_columns,
                strings.date.len() + 1 + strings.from.grapheme_width(),
            );
            rows.push(strings);
            if refresh_mailbox {
                self.all_threads.insert(root_idx);
            }

            self.order.insert(root_idx, idx);
            self.selection.insert(root_idx, false);
        }
        let ConversationsListing {
            ref mut selection,
            ref order,
            ..
        } = self;
        selection.retain(|e, _| order.contains_key(e));

        let width = max_entry_columns;
        self.content =
            CellBuffer::new_with_context(width, 4 * rows.len(), Cell::with_char(' '), context);

        let padding_fg = if context.settings.terminal.theme == "light" {
            Color::Byte(254)
        } else {
            Color::Byte(235)
        };
        let threads_iter = if self.filter_term.is_empty() {
            Box::new(threads.root_iter()) as Box<dyn Iterator<Item = ThreadHash>>
        } else {
            Box::new(self.filtered_selection.iter().map(|h| *h))
                as Box<dyn Iterator<Item = ThreadHash>>
        };

        for ((idx, root_idx), strings) in threads_iter.enumerate().zip(rows) {
            let thread_node = &threads.thread_nodes()[&root_idx];
            let i = thread_node.message().unwrap_or_else(|| {
                let mut iter_ptr = thread_node.children()[0];
                while threads.thread_nodes()[&iter_ptr].message().is_none() {
                    iter_ptr = threads.thread_nodes()[&iter_ptr].children()[0];
                }
                threads.thread_nodes()[&iter_ptr].message().unwrap()
            });
            if !context.accounts[self.cursor_pos.0].contains_key(i) {
                panic!();
            }
            let fg_color = if thread_node.has_unseen() {
                Color::Byte(0)
            } else {
                Color::Default
            };
            let bg_color = if thread_node.has_unseen() {
                Color::Byte(251)
            } else {
                Color::Default
            };
            /* draw flags */
            let (x, _) = write_string_to_grid(
                &strings.flag,
                &mut self.content,
                fg_color,
                bg_color,
                Attr::Default,
                ((0, 3 * idx), (width - 1, 3 * idx)),
                None,
            );
            for x in x..(x + 3) {
                self.content[(x, 3 * idx)].set_bg(bg_color);
            }
            /* draw subject */
            let (mut x, _) = write_string_to_grid(
                &strings.subject,
                &mut self.content,
                fg_color,
                bg_color,
                Attr::Bold,
                ((x, 3 * idx), (width - 1, 3 * idx)),
                None,
            );
            for (t, &color) in strings.tags.split_whitespace().zip(strings.tags.1.iter()) {
                let (_x, _) = write_string_to_grid(
                    t,
                    &mut self.content,
                    Color::White,
                    color,
                    Attr::Bold,
                    ((x + 1, 3 * idx), (width - 1, 3 * idx)),
                    None,
                );
                self.content[(x, 3 * idx)].set_bg(color);
                if _x < width {
                    self.content[(_x, 3 * idx)].set_bg(color);
                    self.content[(_x, 3 * idx)].set_keep_bg(true);
                }
                for x in (x + 1).._x {
                    self.content[(x, 3 * idx)].set_keep_fg(true);
                    self.content[(x, 3 * idx)].set_keep_bg(true);
                }
                self.content[(x, 3 * idx)].set_keep_bg(true);
                x = _x + 1;
            }
            for x in x..width {
                self.content[(x, 3 * idx)].set_ch(' ');
                self.content[(x, 3 * idx)].set_bg(bg_color);
            }
            /* Next line, draw date */
            let (x, _) = write_string_to_grid(
                &strings.date,
                &mut self.content,
                fg_color,
                bg_color,
                Attr::Default,
                ((0, 3 * idx + 1), (width - 1, 3 * idx + 1)),
                None,
            );
            for x in x..(x + 4) {
                self.content[(x, 3 * idx + 1)].set_ch('▁');
                self.content[(x, 3 * idx + 1)].set_bg(bg_color);
            }
            /* draw from */
            let (x, _) = write_string_to_grid(
                &strings.from,
                &mut self.content,
                fg_color,
                bg_color,
                Attr::Default,
                ((x + 4, 3 * idx + 1), (width - 1, 3 * idx + 1)),
                None,
            );

            for x in x..width {
                self.content[(x, 3 * idx + 1)].set_ch('▁');
                self.content[(x, 3 * idx + 1)].set_bg(bg_color);
            }
            for x in 0..width {
                self.content[(x, 3 * idx + 2)].set_ch('▓');
                self.content[(x, 3 * idx + 2)].set_fg(padding_fg);
                self.content[(x, 3 * idx + 2)].set_bg(bg_color);
            }
        }
        if self.length == 0 && self.filter_term.is_empty() {
            let mailbox = &account[self.cursor_pos.1];
            let message = mailbox.to_string();
            self.content =
                CellBuffer::new_with_context(message.len(), 1, Cell::with_char(' '), context);
            write_string_to_grid(
                &message,
                &mut self.content,
                Color::Default,
                Color::Default,
                Attr::Default,
                ((0, 0), (message.len() - 1, 0)),
                None,
            );
        }
    }

    pub(super) fn format_date(thread_node: &ThreadNode) -> String {
        let d = std::time::UNIX_EPOCH + std::time::Duration::from_secs(thread_node.date());
        let now: std::time::Duration = std::time::SystemTime::now()
            .duration_since(d)
            .unwrap_or_else(|_| std::time::Duration::new(std::u64::MAX, 0));
        match now.as_secs() {
            n if n < 60 * 60 => format!(
                "{} minute{} ago",
                n / (60),
                if n / 60 == 1 { "" } else { "s" }
            ),
            n if n < 24 * 60 * 60 => format!(
                "{} hour{} ago",
                n / (60 * 60),
                if n / (60 * 60) == 1 { "" } else { "s" }
            ),
            n if n < 7 * 24 * 60 * 60 => format!(
                "{} day{} ago",
                n / (24 * 60 * 60),
                if n / (24 * 60 * 60) == 1 { "" } else { "s" }
            ),
            _ => thread_node
                .datetime()
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        }
    }

    fn get_thread_under_cursor(&self, cursor: usize, context: &Context) -> ThreadHash {
        let account = &context.accounts[self.cursor_pos.0];
        let folder_hash = account[self.cursor_pos.1].unwrap().folder.hash();
        let threads = &account.collection.threads[&folder_hash];
        if self.filter_term.is_empty() {
            threads.root_set(cursor)
        } else {
            self.filtered_selection[cursor]
        }
    }

    fn update_line(&mut self, context: &Context, thread_hash: ThreadHash) {
        let account = &context.accounts[self.cursor_pos.0];
        let folder_hash = account[self.cursor_pos.1].unwrap().folder.hash();
        let threads = &account.collection.threads[&folder_hash];
        let thread_node = &threads.thread_nodes[&thread_hash];
        let idx: usize = self.order[&thread_hash];
        let width = self.content.size().0;

        let fg_color = if thread_node.has_unseen() {
            Color::Byte(0)
        } else {
            Color::Default
        };
        let bg_color = if thread_node.has_unseen() {
            Color::Byte(251)
        } else {
            Color::Default
        };
        let padding_fg = if context.settings.terminal.theme == "light" {
            Color::Byte(254)
        } else {
            Color::Byte(235)
        };
        let mut from_address_list = Vec::new();
        let mut from_address_set: std::collections::HashSet<Vec<u8>> =
            std::collections::HashSet::new();
        let mut stack = StackVec::new();
        stack.push(thread_hash);
        while let Some(h) = stack.pop() {
            let env_hash = if let Some(h) = threads.thread_nodes()[&h].message() {
                h
            } else {
                break;
            };

            let envelope: &EnvelopeRef = &context.accounts[self.cursor_pos.0]
                .collection
                .get_env(env_hash);
            for addr in envelope.from().iter() {
                if from_address_set.contains(addr.raw()) {
                    continue;
                }
                from_address_set.insert(addr.raw().to_vec());
                from_address_list.push(addr.clone());
            }
            for c in threads.thread_nodes()[&h].children() {
                stack.push(*c);
            }
        }
        let env_hash = threads[&thread_hash].message().unwrap();
        let envelope: EnvelopeRef = account.collection.get_env(env_hash);
        let strings = self.make_entry_string(
            &envelope,
            context,
            &from_address_list,
            &threads[&thread_hash],
            threads.is_snoozed(thread_hash),
        );
        drop(envelope);
        /* draw flags */
        let (x, _) = write_string_to_grid(
            &strings.flag,
            &mut self.content,
            fg_color,
            bg_color,
            Attr::Default,
            ((0, 3 * idx), (width - 1, 3 * idx)),
            None,
        );
        for c in self.content.row_iter(x..(x + 4), 3 * idx) {
            self.content[c].set_bg(bg_color);
        }
        /* draw subject */
        let (x, _) = write_string_to_grid(
            &strings.subject,
            &mut self.content,
            fg_color,
            bg_color,
            Attr::Bold,
            ((x, 3 * idx), (width - 1, 3 * idx)),
            None,
        );
        let x = {
            let mut x = x + 1;
            for (t, &color) in strings.tags.split_whitespace().zip(strings.tags.1.iter()) {
                let (_x, _) = write_string_to_grid(
                    t,
                    &mut self.content,
                    Color::White,
                    color,
                    Attr::Bold,
                    ((x + 1, 3 * idx), (width - 1, 3 * idx)),
                    None,
                );
                for c in self.content.row_iter(x..(x + 1), 3 * idx) {
                    self.content[c].set_bg(color);
                }
                for c in self.content.row_iter(_x..(_x + 1), 3 * idx) {
                    self.content[c].set_bg(color);
                    self.content[c].set_keep_bg(true);
                }
                for c in self.content.row_iter(x + 1..(_x + 1), 3 * idx) {
                    self.content[c].set_keep_fg(true);
                    self.content[c].set_keep_bg(true);
                }
                for c in self.content.row_iter(x..(x + 1), 3 * idx) {
                    self.content[c].set_keep_bg(true);
                }
                x = _x + 1;
            }
            x
        };
        for c in self.content.row_iter(x..width, 3 * idx) {
            self.content[c].set_ch(' ');
            self.content[c].set_bg(bg_color);
        }
        /* Next line, draw date */
        let (x, _) = write_string_to_grid(
            &strings.date,
            &mut self.content,
            fg_color,
            bg_color,
            Attr::Default,
            ((0, 3 * idx + 1), (width - 1, 3 * idx + 1)),
            None,
        );
        for c in self.content.row_iter(x..(x + 5), 3 * idx + 1) {
            self.content[c].set_ch('▁');
            self.content[c].set_bg(bg_color);
        }
        /* draw from */
        let (x, _) = write_string_to_grid(
            &strings.from,
            &mut self.content,
            fg_color,
            bg_color,
            Attr::Default,
            ((x + 4, 3 * idx + 1), (width - 1, 3 * idx + 1)),
            None,
        );

        for c in self.content.row_iter(x..width, 3 * idx + 1) {
            self.content[c].set_ch('▁');
            self.content[c].set_bg(bg_color);
        }
        for c in self.content.row_iter(0..width, 3 * idx + 2) {
            self.content[c].set_ch('▓');
            self.content[c].set_fg(padding_fg);
            self.content[c].set_bg(bg_color);
        }
    }
}

impl Component for ConversationsListing {
    fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if !self.is_dirty() {
            return;
        }
        let (upper_left, bottom_right) = area;
        {
            let mut area = if self.unfocused {
                clear_area(
                    grid,
                    (
                        pos_inc(upper_left, (width!(area) / 3, 0)),
                        set_x(bottom_right, get_x(upper_left) + width!(area) / 3 + 1),
                    ),
                );
                context.dirty_areas.push_back((
                    pos_inc(upper_left, (width!(area) / 3, 0)),
                    set_x(bottom_right, get_x(upper_left) + width!(area) / 3 + 1),
                ));
                (
                    upper_left,
                    set_x(bottom_right, get_x(upper_left) + width!(area) / 3 - 1),
                )
            } else {
                area
            };

            if !self.filter_term.is_empty() {
                let (x, y) = write_string_to_grid(
                    &format!(
                        "{} results for `{}` (Press ESC to exit)",
                        self.filtered_selection.len(),
                        self.filter_term
                    ),
                    grid,
                    Color::Default,
                    Color::Default,
                    Attr::Default,
                    area,
                    Some(get_x(upper_left)),
                );
                for c in grid.row_iter(x..(get_x(bottom_right) + 1), y) {
                    grid[c] = Cell::default();
                }
                clear_area(grid, ((x, y), set_y(bottom_right, y)));
                context
                    .dirty_areas
                    .push_back((upper_left, set_y(bottom_right, y + 1)));

                area = (set_y(upper_left, y + 1), bottom_right);
            }
            if !self.row_updates.is_empty() {
                /* certain rows need to be updated (eg an unseen message was just set seen)
                 * */
                let (upper_left, bottom_right) = area;
                while let Some(row) = self.row_updates.pop() {
                    self.update_line(context, row);
                    let row: usize = self.order[&row];

                    let rows = (get_y(bottom_right) - get_y(upper_left) + 1) / 3;
                    let page_no = (self.cursor_pos.2).wrapping_div(rows);

                    let top_idx = page_no * rows;
                    /* Update row only if it's currently visible */
                    if row >= top_idx && row <= top_idx + rows {
                        let area = (
                            set_y(upper_left, get_y(upper_left) + (3 * (row % rows))),
                            set_y(bottom_right, get_y(upper_left) + (3 * (row % rows) + 2)),
                        );
                        self.highlight_line(grid, area, row, context);
                        context.dirty_areas.push_back(area);
                    }
                }
                if self.force_draw {
                    /* Draw the entire list */
                    self.draw_list(grid, area, context);
                    self.force_draw = false;
                }
            } else {
                /* Draw the entire list */
                self.draw_list(grid, area, context);
            }
        }
        if self.unfocused {
            if self.length == 0 && self.dirty {
                clear_area(grid, area);
                context.dirty_areas.push_back(area);
                return;
            }

            let area = (
                set_x(upper_left, get_x(upper_left) + width!(area) / 3 + 2),
                bottom_right,
            );
            self.view.draw(grid, area, context);
        }
        self.dirty = false;
    }
    fn process_event(&mut self, event: &mut UIEvent, context: &mut Context) -> bool {
        if self.unfocused && self.view.process_event(event, context) {
            return true;
        }

        let shortcuts = self.get_shortcuts(context);
        if self.length > 0 {
            match *event {
                UIEvent::Input(ref k)
                    if !self.unfocused
                        && shortcut!(
                            k == shortcuts[ConversationsListing::DESCRIPTION]["open_thread"]
                        ) =>
                {
                    if self.length == 0 {
                        return true;
                    }

                    if self.filter_term.is_empty() {
                        self.view = ThreadView::new(self.cursor_pos, None, context);
                    } else if !self.filtered_selection.is_empty() {
                        let mut temp = self.cursor_pos;
                        let thread_hash = self.get_thread_under_cursor(self.cursor_pos.2, context);
                        let account = &context.accounts[self.cursor_pos.0];
                        let folder_hash = account[self.cursor_pos.1].unwrap().folder.hash();
                        let threads = &account.collection.threads[&folder_hash];
                        let root_thread_index = threads.root_iter().position(|t| t == thread_hash);
                        if let Some(pos) = root_thread_index {
                            temp.2 = pos;
                            self.view = ThreadView::new(temp, Some(thread_hash), context);
                        } else {
                            return true;
                        }
                    }

                    self.unfocused = true;
                    self.dirty = true;
                    return true;
                }
                UIEvent::Input(ref k)
                    if self.unfocused
                        && shortcut!(
                            k == shortcuts[ConversationsListing::DESCRIPTION]["exit_thread"]
                        ) =>
                {
                    self.unfocused = false;
                    self.dirty = true;
                    /* If self.row_updates is not empty and we exit a thread, the row_update events
                     * will be performed but the list will not be drawn. So force a draw in any case.
                     * */
                    self.force_draw = true;
                    return true;
                }
                UIEvent::Input(ref key)
                    if !self.unfocused
                        && shortcut!(
                            key == shortcuts[ConversationsListing::DESCRIPTION]["select_entry"]
                        ) =>
                {
                    let thread_hash = self.get_thread_under_cursor(self.cursor_pos.2, context);
                    self.selection.entry(thread_hash).and_modify(|e| *e = !*e);
                    return true;
                }
                UIEvent::EnvelopeRename(ref old_hash, ref new_hash) => {
                    let account = &context.accounts[self.cursor_pos.0];
                    let folder_hash = account[self.cursor_pos.1].unwrap().folder.hash();
                    let threads = &account.collection.threads[&folder_hash];
                    if !account.collection.contains_key(&new_hash) {
                        return false;
                    }
                    let new_env_thread_hash = account.collection.get_env(*new_hash).thread();
                    if !threads.thread_nodes.contains_key(&new_env_thread_hash) {
                        return false;
                    }
                    let thread_group = melib::find_root_hash(
                        &threads.thread_nodes,
                        threads.thread_nodes[&new_env_thread_hash].thread_group(),
                    );
                    let (&thread_hash, &row): (&ThreadHash, &usize) = self
                        .order
                        .iter()
                        .find(|(n, _)| {
                            melib::find_root_hash(
                                &threads.thread_nodes,
                                threads.thread_nodes[&n].thread_group(),
                            ) == thread_group
                        })
                        .unwrap();

                    let new_thread_hash = threads.root_set(row);
                    self.row_updates.push(new_thread_hash);
                    if let Some(row) = self.order.remove(&thread_hash) {
                        self.order.insert(new_thread_hash, row);
                        let selection_status = self.selection.remove(&thread_hash).unwrap();
                        self.selection.insert(new_thread_hash, selection_status);
                        for h in self.filtered_selection.iter_mut() {
                            if *h == thread_hash {
                                *h = new_thread_hash;
                                break;
                            }
                        }
                    }

                    self.dirty = true;

                    self.view
                        .process_event(&mut UIEvent::EnvelopeRename(*old_hash, *new_hash), context);
                }
                UIEvent::Action(ref action) => match action {
                    Action::SubSort(field, order) if !self.unfocused => {
                        debug!("SubSort {:?} , {:?}", field, order);
                        self.subsort = (*field, *order);
                        //if !self.filtered_selection.is_empty() {
                        //    let threads = &account.collection.threads[&folder_hash];
                        //    threads.vec_inner_sort_by(&mut self.filtered_selection, self.sort, &account.collection);
                        //} else {
                        //    self.refresh_mailbox(context);
                        //}
                        return true;
                    }
                    Action::Sort(field, order) if !self.unfocused => {
                        debug!("Sort {:?} , {:?}", field, order);
                        self.sort = (*field, *order);
                        if !self.filtered_selection.is_empty() {
                            let folder_hash = context.accounts[self.cursor_pos.0]
                                [self.cursor_pos.1]
                                .unwrap()
                                .folder
                                .hash();
                            let threads = &context.accounts[self.cursor_pos.0].collection.threads
                                [&folder_hash];
                            threads.vec_inner_sort_by(
                                &mut self.filtered_selection,
                                self.sort,
                                &context.accounts[self.cursor_pos.0].collection.envelopes,
                            );
                            self.dirty = true;
                        } else {
                            self.refresh_mailbox(context);
                        }
                        return true;
                    }
                    Action::ToggleThreadSnooze if !self.unfocused => {
                        let thread_hash = self.get_thread_under_cursor(self.cursor_pos.2, context);
                        let account = &mut context.accounts[self.cursor_pos.0];
                        let folder_hash = account[self.cursor_pos.1].unwrap().folder.hash();
                        let threads = account.collection.threads.entry(folder_hash).or_default();
                        let root_node = threads.thread_nodes.entry(thread_hash).or_default();
                        let is_snoozed = root_node.snoozed();
                        root_node.set_snoozed(!is_snoozed);
                        self.row_updates.push(thread_hash);
                        self.refresh_mailbox(context);
                        return true;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        match *event {
            UIEvent::MailboxUpdate((ref idxa, ref idxf))
                if context.accounts[self.new_cursor_pos.0]
                    .folders_order
                    .get(self.new_cursor_pos.1)
                    .map(|&folder_hash| (*idxa, *idxf) == (self.new_cursor_pos.0, folder_hash))
                    .unwrap_or(false) =>
            {
                self.refresh_mailbox(context);
                self.set_dirty(true);
            }
            UIEvent::StartupCheck(ref f)
                if context.accounts[self.new_cursor_pos.0]
                    .folders_order
                    .get(self.new_cursor_pos.1)
                    .map(|&folder_hash| *f == folder_hash)
                    .unwrap_or(false) =>
            {
                self.refresh_mailbox(context);
                self.set_dirty(true);
            }
            UIEvent::ChangeMode(UIMode::Normal) => {
                self.dirty = true;
            }
            UIEvent::Resize => {
                self.dirty = true;
            }
            UIEvent::Action(ref action) => match action {
                Action::ViewMailbox(idx) => {
                    if context.accounts[self.cursor_pos.0]
                        .folders_order
                        .get(*idx)
                        .is_none()
                    {
                        return true;
                    }
                    self.set_coordinates((self.new_cursor_pos.0, *idx));
                    self.refresh_mailbox(context);
                    return true;
                }

                Action::Listing(Filter(ref filter_term)) if !self.unfocused => {
                    self.filter(filter_term, context);
                    self.dirty = true;
                    return true;
                }
                _ => {}
            },
            UIEvent::Input(Key::Esc) | UIEvent::Input(Key::Char(''))
                if !self.unfocused && !&self.filter_term.is_empty() =>
            {
                self.set_coordinates((self.new_cursor_pos.0, self.new_cursor_pos.1));
                self.refresh_mailbox(context);
                self.force_draw = false;
                self.set_dirty(true);
                return true;
            }
            _ => {}
        }

        false
    }
    fn is_dirty(&self) -> bool {
        self.dirty
            || if self.unfocused {
                self.view.is_dirty()
            } else {
                false
            }
    }
    fn set_dirty(&mut self, value: bool) {
        if self.unfocused {
            self.view.set_dirty(value);
        }
        self.dirty = value;
    }

    fn get_shortcuts(&self, context: &Context) -> ShortcutMaps {
        let mut map = if self.unfocused {
            self.view.get_shortcuts(context)
        } else {
            ShortcutMaps::default()
        };

        let config_map = context.settings.shortcuts.compact_listing.key_values();
        map.insert(ConversationsListing::DESCRIPTION, config_map);

        map
    }

    fn id(&self) -> ComponentId {
        self.id
    }
    fn set_id(&mut self, id: ComponentId) {
        self.id = id;
    }
}

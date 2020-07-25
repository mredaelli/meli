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

/*!
 * User actions that need to be handled by the UI
 */

use crate::components::Component;
use melib::backends::MailboxHash;
pub use melib::thread::{SortField, SortOrder};
use melib::{Draft, EnvelopeHash};

extern crate uuid;
use uuid::Uuid;

#[derive(Debug)]
pub enum TagAction {
    Add(String),
    Remove(String),
}

#[derive(Debug)]
pub enum ListingAction {
    SetPlain,
    SetThreaded,
    SetCompact,
    SetConversations,
    Search(String),
    Select(String),
    SetSeen,
    SetUnseen,
    CopyTo(MailboxPath),
    CopyToOtherAccount(AccountName, MailboxPath),
    MoveTo(MailboxPath),
    MoveToOtherAccount(AccountName, MailboxPath),
    Delete,
    OpenInNewTab,
    Tag(TagAction),
}

#[derive(Debug)]
pub enum TabAction {
    New(Option<Box<dyn Component>>),
    NewDraft(usize, Option<Draft>),
    Reply((usize, MailboxHash), EnvelopeHash), // thread coordinates (account, mailbox) and envelope
    Close,
    Edit(usize, EnvelopeHash), // account_position, envelope hash
    Kill(Uuid),
}

#[derive(Debug)]
pub enum MailingListAction {
    ListPost,
    ListArchive,
    ListUnsubscribe,
}

#[derive(Debug)]
pub enum ViewAction {
    Pipe(String, Vec<String>),
    SaveAttachment(usize, String),
}

#[derive(Debug)]
pub enum ComposeAction {
    AddAttachment(String),
    AddAttachmentPipe(String),
    RemoveAttachment(usize),
    SaveDraft,
    ToggleSign,
}

#[derive(Debug)]
pub enum AccountAction {
    ReIndex,
    PrintAccountSetting(String),
}

#[derive(Debug)]
pub enum MailboxOperation {
    Create(NewMailboxPath),
    Delete(MailboxPath),
    Subscribe(MailboxPath),
    Unsubscribe(MailboxPath),
    Rename(MailboxPath, NewMailboxPath),
    // Placeholder
    SetPermissions(MailboxPath),
}

#[derive(Debug)]
pub enum Action {
    Listing(ListingAction),
    ViewMailbox(usize),
    Sort(SortField, SortOrder),
    SubSort(SortField, SortOrder),
    Tab(TabAction),
    ToggleThreadSnooze,
    MailingListAction(MailingListAction),
    View(ViewAction),
    SetEnv(String, String),
    PrintEnv(String),
    Compose(ComposeAction),
    Mailbox(AccountName, MailboxOperation),
    AccountAction(AccountName, AccountAction),
    PrintSetting(String),
}

impl Action {
    pub fn needs_confirmation(&self) -> bool {
        match self {
            Action::Listing(_) => false,
            Action::ViewMailbox(_) => false,
            Action::Sort(_, _) => false,
            Action::SubSort(_, _) => false,
            Action::Tab(_) => false,
            Action::ToggleThreadSnooze => false,
            Action::MailingListAction(_) => true,
            Action::View(_) => false,
            Action::SetEnv(_, _) => false,
            Action::PrintEnv(_) => false,
            Action::Compose(_) => false,
            Action::Mailbox(_, _) => true,
            Action::AccountAction(_, _) => false,
            Action::PrintSetting(_) => false,
        }
    }
}

type AccountName = String;
type MailboxPath = String;
type NewMailboxPath = String;
/*
 * meli - configuration module.
 *
 * Copyright 2017 Manos Pitsidianakis
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

/*! Configuration logic and `config.toml` interfaces. */

extern crate bincode;
extern crate serde;
extern crate toml;
extern crate xdg;

use crate::conf::deserializers::non_empty_string;
use crate::terminal::Color;
use melib::search::Query;
use std::collections::HashSet;
mod overrides;
pub use overrides::*;
pub mod composing;
pub mod notifications;
pub mod pager;
pub mod pgp;
pub mod tags;
#[macro_use]
pub mod shortcuts;
pub mod bindings;
mod listing;
pub mod terminal;
mod themes;
pub use themes::*;

pub mod accounts;
pub use self::accounts::Account;
pub use self::bindings::*;
pub use self::composing::*;
pub use self::pgp::*;
pub use self::shortcuts::*;
pub use self::tags::*;

use self::default_vals::*;
use self::listing::ListingSettings;
use self::notifications::NotificationsSettings;
use self::terminal::TerminalSettings;
use crate::pager::PagerSettings;
use melib::conf::{AccountSettings, MailboxConf, ToggleFlag};
use melib::error::*;

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use indexmap::IndexMap;
use std::collections::HashMap;
use std::env;
use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[macro_export]
macro_rules! split_command {
    ($cmd:expr) => {{
        $cmd.split_whitespace().collect::<Vec<&str>>()
    }};
}

#[macro_export]
macro_rules! account_settings {
    ($context:ident[$account_hash:expr].$setting:ident.$field:ident) => {{
        $context.accounts[&$account_hash]
            .settings
            .conf_override
            .$setting
            .$field
            .as_ref()
            .unwrap_or(&$context.settings.$setting.$field)
    }};
}
#[macro_export]
macro_rules! mailbox_settings {
    ($context:ident[$account_hash:expr][$mailbox_path:expr].$setting:ident.$field:ident) => {{
        $context.accounts[&$account_hash][$mailbox_path]
            .conf
            .conf_override
            .$setting
            .$field
            .as_ref()
            .or($context.accounts[&$account_hash]
                .settings
                .conf_override
                .$setting
                .$field
                .as_ref())
            .unwrap_or(&$context.settings.$setting.$field)
    }};
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MailUIConf {
    #[serde(default)]
    pub pager: PagerSettingsOverride,
    #[serde(default)]
    pub listing: ListingSettingsOverride,
    #[serde(default)]
    pub notifications: NotificationsSettingsOverride,
    #[serde(default)]
    pub shortcuts: ShortcutsOverride,
    #[serde(default)]
    pub composing: ComposingSettingsOverride,
    #[serde(default)]
    pub identity: Option<String>,
    #[serde(default)]
    pub tags: TagsSettingsOverride,
    #[serde(default)]
    pub themes: Option<Themes>,
    #[serde(default)]
    pub pgp: PGPSettingsOverride,
}

#[serde(default)]
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct FileMailboxConf {
    #[serde(flatten)]
    pub conf_override: MailUIConf,
    #[serde(flatten)]
    pub mailbox_conf: MailboxConf,
}

impl FileMailboxConf {
    pub fn conf_override(&self) -> &MailUIConf {
        &self.conf_override
    }

    pub fn mailbox_conf(&self) -> &MailboxConf {
        &self.mailbox_conf
    }
}

use crate::conf::deserializers::extra_settings;
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileAccount {
    root_mailbox: String,
    format: String,
    identity: String,
    #[serde(default = "none")]
    display_name: Option<String>,

    #[serde(default = "false_val")]
    read_only: bool,
    #[serde(default)]
    subscribed_mailboxes: Vec<String>,
    #[serde(default)]
    mailboxes: IndexMap<String, FileMailboxConf>,
    #[serde(default)]
    search_backend: SearchBackend,
    #[serde(default = "false_val")]
    pub manual_refresh: bool,
    #[serde(default = "none")]
    pub refresh_command: Option<String>,
    #[serde(flatten)]
    pub conf_override: MailUIConf,
    #[serde(flatten)]
    #[serde(deserialize_with = "extra_settings")]
    pub extra: IndexMap<String, String>, /* use custom deserializer to convert any given value (eg bool, number, etc) to string */
}

impl FileAccount {
    pub fn mailboxes(&self) -> &IndexMap<String, FileMailboxConf> {
        &self.mailboxes
    }

    pub fn mailbox(&self) -> &str {
        &self.root_mailbox
    }

    pub fn search_backend(&self) -> &SearchBackend {
        &self.search_backend
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileSettings {
    pub accounts: IndexMap<String, FileAccount>,
    #[serde(default)]
    pub pager: PagerSettings,
    #[serde(default)]
    pub listing: ListingSettings,
    #[serde(default)]
    pub notifications: NotificationsSettings,
    #[serde(default)]
    pub shortcuts: Shortcuts,
    pub composing: ComposingSettings,
    #[serde(default)]
    pub tags: TagsSettings,
    #[serde(default)]
    pub pgp: PGPSettings,
    #[serde(default)]
    pub terminal: TerminalSettings,
    #[serde(default)]
    pub log: LogSettings,
    pub bindings: Bindings,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AccountConf {
    pub account: AccountSettings,
    pub conf: FileAccount,
    pub conf_override: MailUIConf,
    pub mailbox_confs: IndexMap<String, FileMailboxConf>,
}

impl AccountConf {
    pub fn account(&self) -> &AccountSettings {
        &self.account
    }
    pub fn account_mut(&mut self) -> &mut AccountSettings {
        &mut self.account
    }
    pub fn conf(&self) -> &FileAccount {
        &self.conf
    }
    pub fn conf_mut(&mut self) -> &mut FileAccount {
        &mut self.conf
    }
}

impl From<FileAccount> for AccountConf {
    fn from(x: FileAccount) -> Self {
        let format = x.format.to_lowercase();
        let root_mailbox = x.root_mailbox.clone();
        let identity = x.identity.clone();
        let display_name = x.display_name.clone();
        let mailboxes = x
            .mailboxes
            .iter()
            .map(|(k, v)| (k.clone(), v.mailbox_conf.clone()))
            .collect();

        let acc = AccountSettings {
            name: String::new(),
            root_mailbox,
            format,
            identity,
            read_only: x.read_only,
            display_name,
            subscribed_mailboxes: x.subscribed_mailboxes.clone(),
            mailboxes,
            manual_refresh: x.manual_refresh,
            extra: x.extra.clone().into_iter().collect(),
        };

        let mailbox_confs = x.mailboxes.clone();
        AccountConf {
            account: acc,
            conf_override: x.conf_override.clone(),
            conf: x,
            mailbox_confs,
        }
    }
}

pub fn get_config_file() -> Result<PathBuf> {
    let xdg_dirs = xdg::BaseDirectories::with_prefix("meli").map_err(|err| {
        MeliError::new(format!(
            "Could not detect XDG directories for user: {}",
            err
        ))
        .set_source(Some(std::sync::Arc::new(Box::new(err))))
    })?;
    match env::var("MELI_CONFIG") {
        Ok(path) => Ok(PathBuf::from(path)),
        Err(_) => Ok(xdg_dirs
            .place_config_file("config.toml")
            .chain_err_summary(|| {
                format!(
                    "Cannot create configuration directory in {}",
                    xdg_dirs.get_config_home().display()
                )
            })?),
    }
}

impl FileSettings {
    pub fn new() -> Result<FileSettings> {
        let config_path = get_config_file()?;
        if !config_path.exists() {
            let path_string = config_path.display().to_string();
            if path_string.is_empty() {
                return Err(MeliError::new("No configuration found."));
            }
            println!(
                "No configuration found. Would you like to generate one in {}? [Y/n]",
                path_string
            );
            let mut buffer = String::new();
            let stdin = io::stdin();
            let mut handle = stdin.lock();

            loop {
                buffer.clear();
                handle
                    .read_line(&mut buffer)
                    .expect("Could not read from stdin.");

                match buffer.trim() {
                    "" | "Y" | "y" | "yes" | "YES" | "Yes" => {
                        create_config_file(&config_path)?;
                        return Err(MeliError::new(
                            "Edit the sample configuration and relaunch meli.",
                        ));
                    }
                    "n" | "N" | "no" | "No" | "NO" => {
                        return Err(MeliError::new("No configuration file found."));
                    }
                    _ => {
                        println!(
                            "No configuration found. Would you like to generate one in {}? [Y/n]",
                            path_string
                        );
                    }
                }
            }
        }

        FileSettings::validate(config_path)
    }

    pub fn validate(path: PathBuf) -> Result<Self> {
        let s = pp::pp(&path)?;
        let mut s: FileSettings = toml::from_str(&s).map_err(|e| {
            MeliError::new(format!(
                "{}:\nConfig file contains errors: {}",
                path.display(),
                e.to_string()
            ))
        })?;
        let backends = melib::backends::Backends::new();
        let Themes {
            light: default_light,
            dark: default_dark,
            ..
        } = Themes::default();
        for (k, v) in default_light.keys.into_iter() {
            if !s.terminal.themes.light.contains_key(&k) {
                s.terminal.themes.light.insert(k, v);
            }
        }
        for theme in s.terminal.themes.other_themes.values_mut() {
            for (k, v) in default_dark.keys.clone().into_iter() {
                if !theme.contains_key(&k) {
                    theme.insert(k, v);
                }
            }
        }
        for (k, v) in default_dark.keys.into_iter() {
            if !s.terminal.themes.dark.contains_key(&k) {
                s.terminal.themes.dark.insert(k, v);
            }
        }
        match s.terminal.theme.as_str() {
            "dark" | "light" => {}
            t if s.terminal.themes.other_themes.contains_key(t) => {}
            t => {
                return Err(MeliError::new(format!("Theme `{}` was not found.", t)));
            }
        }

        s.terminal.themes.validate()?;
        for (name, acc) in &s.accounts {
            let FileAccount {
                root_mailbox,
                format,
                identity,
                read_only,
                display_name,
                subscribed_mailboxes,
                mailboxes,
                extra,
                manual_refresh,
                refresh_command: _,
                search_backend: _,
                conf_override: _,
            } = acc.clone();

            let lowercase_format = format.to_lowercase();
            let s = AccountSettings {
                name: name.to_string(),
                root_mailbox,
                format: format.clone(),
                identity,
                read_only,
                display_name,
                subscribed_mailboxes,
                manual_refresh,
                mailboxes: mailboxes
                    .into_iter()
                    .map(|(k, v)| (k, v.mailbox_conf))
                    .collect(),
                extra: extra.into_iter().collect(),
            };
            backends.validate_config(&lowercase_format, &s)?;
        }

        Ok(s)
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Settings {
    pub accounts: IndexMap<String, AccountConf>,
    pub pager: PagerSettings,
    pub listing: ListingSettings,
    pub notifications: NotificationsSettings,
    pub shortcuts: Shortcuts,
    pub tags: TagsSettings,
    pub composing: ComposingSettings,
    pub pgp: PGPSettings,
    pub terminal: TerminalSettings,
    pub log: LogSettings,
    pub bindings: Bindings,
}

impl Settings {
    pub fn new() -> Result<Settings> {
        let fs = FileSettings::new()?;
        let mut s: IndexMap<String, AccountConf> = IndexMap::new();

        for (id, x) in fs.accounts {
            let mut ac = AccountConf::from(x);
            ac.account.set_name(id.clone());

            s.insert(id, ac);
        }

        if let Some(ref log_path) = fs.log.log_file {
            melib::change_log_dest(log_path.into());
        }
        if fs.log.maximum_level != melib::LoggingLevel::default() {
            melib::change_log_level(fs.log.maximum_level);
        }

        Ok(Settings {
            accounts: s,
            pager: fs.pager,
            listing: fs.listing,
            notifications: fs.notifications,
            shortcuts: fs.shortcuts,
            tags: fs.tags,
            composing: fs.composing,
            pgp: fs.pgp,
            terminal: fs.terminal,
            log: fs.log,
            bindings: fs.bindings,
        })
    }

    pub fn without_accounts() -> Result<Settings> {
        let fs = FileSettings::new()?;
        if let Some(ref log_path) = fs.log.log_file {
            melib::change_log_dest(log_path.into());
        }
        if fs.log.maximum_level != melib::LoggingLevel::default() {
            melib::change_log_level(fs.log.maximum_level);
        }

        Ok(Settings {
            accounts: IndexMap::new(),
            pager: fs.pager,
            listing: fs.listing,
            notifications: fs.notifications,
            shortcuts: fs.shortcuts,
            tags: fs.tags,
            composing: fs.composing,
            pgp: fs.pgp,
            terminal: fs.terminal,
            log: fs.log,
            bindings: fs.bindings,
        })
    }
}

#[derive(Copy, Debug, Clone, Hash, PartialEq)]
pub enum IndexStyle {
    Plain,
    Threaded,
    Compact,
    Conversations,
}

impl Default for IndexStyle {
    fn default() -> Self {
        IndexStyle::Compact
    }
}

/*
 * Deserialize default functions
 */

mod default_vals {
    pub(in crate::conf) fn false_val<T: std::convert::From<bool>>() -> T {
        false.into()
    }

    pub(in crate::conf) fn true_val<T: std::convert::From<bool>>() -> T {
        true.into()
    }

    pub(in crate::conf) fn zero_val<T: std::convert::From<usize>>() -> T {
        0.into()
    }

    pub(in crate::conf) fn eighty_val<T: std::convert::From<usize>>() -> T {
        80.into()
    }

    pub(in crate::conf) fn none<T>() -> Option<T> {
        None
    }

    pub(in crate::conf) fn internal_value_false<T: std::convert::From<super::ToggleFlag>>() -> T {
        super::ToggleFlag::InternalVal(false).into()
    }

    pub(in crate::conf) fn internal_value_true<T: std::convert::From<super::ToggleFlag>>() -> T {
        super::ToggleFlag::InternalVal(true).into()
    }
}

mod deserializers {
    use serde::{Deserialize, Deserializer};
    pub(in crate::conf) fn non_empty_string<'de, D, T: std::convert::From<Option<String>>>(
        deserializer: D,
    ) -> std::result::Result<T, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <String>::deserialize(deserializer)?;
        if s.is_empty() {
            Ok(None.into())
        } else {
            Ok(Some(s).into())
        }
    }

    use toml::Value;
    fn any_of<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v: Value = Deserialize::deserialize(deserializer)?;
        let mut ret = v.to_string();
        if ret.starts_with('"') && ret.ends_with('"') {
            ret.drain(0..1).count();
            ret.drain(ret.len() - 1..).count();
        }
        Ok(ret)
    }

    use indexmap::IndexMap;
    pub(in crate::conf) fn extra_settings<'de, D>(
        deserializer: D,
    ) -> std::result::Result<IndexMap<String, String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        /* Why is this needed? If the user gives a configuration value such as key = true, the
         * parsing will fail since it expects string values. We want to accept key = true as well
         * as key = "true". */
        #[derive(Deserialize)]
        struct Wrapper(#[serde(deserialize_with = "any_of")] String);

        let v = <IndexMap<String, Wrapper>>::deserialize(deserializer)?;
        Ok(v.into_iter().map(|(k, Wrapper(v))| (k, v)).collect())
    }
}

impl<'de> Deserialize<'de> for IndexStyle {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <String>::deserialize(deserializer)?;
        match s.as_str() {
            "Plain" | "plain" => Ok(IndexStyle::Plain),
            "Threaded" | "threaded" => Ok(IndexStyle::Threaded),
            "Compact" | "compact" => Ok(IndexStyle::Compact),
            "Conversations" | "conversations" => Ok(IndexStyle::Conversations),
            _ => Err(de::Error::custom("invalid `index_style` value")),
        }
    }
}

impl Serialize for IndexStyle {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            IndexStyle::Plain => serializer.serialize_str("plain"),
            IndexStyle::Threaded => serializer.serialize_str("threaded"),
            IndexStyle::Compact => serializer.serialize_str("compact"),
            IndexStyle::Conversations => serializer.serialize_str("conversations"),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum SearchBackend {
    None,
    Auto,
    #[cfg(feature = "sqlite3")]
    Sqlite3,
}

impl Default for SearchBackend {
    fn default() -> Self {
        SearchBackend::Auto
    }
}

impl<'de> Deserialize<'de> for SearchBackend {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <String>::deserialize(deserializer)?;
        match s.as_str() {
            #[cfg(feature = "sqlite3")]
            sqlite3 if sqlite3.eq_ignore_ascii_case("sqlite3") => Ok(SearchBackend::Sqlite3),
            none if none.eq_ignore_ascii_case("none")
                || none.eq_ignore_ascii_case("nothing")
                || none.is_empty() =>
            {
                Ok(SearchBackend::None)
            }
            auto if auto.eq_ignore_ascii_case("auto") => Ok(SearchBackend::Auto),
            _ => Err(de::Error::custom("invalid `search_backend` value")),
        }
    }
}

impl Serialize for SearchBackend {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            #[cfg(feature = "sqlite3")]
            SearchBackend::Sqlite3 => serializer.serialize_str("sqlite3"),
            SearchBackend::None => serializer.serialize_str("none"),
            SearchBackend::Auto => serializer.serialize_str("auto"),
        }
    }
}

pub fn create_config_file(p: &Path) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(p)
        .expect("Could not create config file.");
    file.write_all(include_bytes!("../docs/samples/sample-config.toml"))
        .expect("Could not write to config file.");
    println!("Written example configuration to {}", p.display());
    let metadata = file.metadata()?;
    let mut permissions = metadata.permissions();

    permissions.set_mode(0o600); // Read/write for owner only.
    file.set_permissions(permissions)?;
    Ok(())
}

mod pp {
    //! Preprocess configuration files by unfolding `include` macros.
    use melib::{
        error::{MeliError, Result},
        parsec::*,
        ShellExpandTrait,
    };
    use std::io::Read;
    use std::path::{Path, PathBuf};

    /// Try to parse line into a path to be included.
    fn include_directive<'a>() -> impl Parser<'a, Option<&'a str>> {
        move |input: &'a str| {
            enum State {
                Start,
                Path,
            }
            use State::*;
            let mut state = State::Start;

            let mut i = 0;
            while i < input.len() {
                match (&state, input.as_bytes()[i]) {
                    (Start, b'#') => {
                        return Ok(("", None));
                    }
                    (Start, b) if (b as char).is_whitespace() => { /* consume */ }
                    (Start, _) if input.as_bytes()[i..].starts_with(b"include(") => {
                        i += "include(".len();
                        state = Path;
                        continue;
                    }
                    (Start, _) => {
                        return Ok(("", None));
                    }
                    (Path, b'"') | (Path, b'\'') | (Path, b'`') => {
                        let mut end = i + 1;
                        while end < input.len() && input.as_bytes()[end] != input.as_bytes()[i] {
                            end += 1;
                        }
                        if end == input.len() {
                            return Err(input);
                        }
                        let ret = &input[i + 1..end];
                        end += 1;
                        if end < input.len() && input.as_bytes()[end] != b')' {
                            /* Nothing else allowed in line */
                            return Err(input);
                        }
                        end += 1;
                        while end < input.len() {
                            if !(input.as_bytes()[end] as char).is_whitespace() {
                                /* Nothing else allowed in line */
                                return Err(input);
                            }
                            end += 1;
                        }
                        return Ok(("", Some(ret)));
                    }
                    (Path, _) => return Err(input),
                }
                i += 1;
            }
            return Ok(("", None));
        }
    }

    /// Expands `include` macros in path.
    fn pp_helper(path: &Path, level: u8) -> Result<String> {
        if level > 7 {
            return Err(MeliError::new(format!("Maximum recursion limit reached while unfolding include directives in {}. Have you included a config file within itself?", path.display())));
        }
        let mut contents = String::new();
        let mut file = std::fs::File::open(path)?;
        file.read_to_string(&mut contents)?;
        let mut ret = String::with_capacity(contents.len());

        for (i, l) in contents.lines().enumerate() {
            if let (_, Some(sub_path)) = include_directive().parse(l).map_err(|l| {
                MeliError::new(format!(
                    "Malformed include directive in line {} of file {}: {}\nConfiguration uses the standard m4 macro include(`filename`).",
                    i,
                    path.display(),
                    l
                ))
            })? {
                let mut p = Path::new(sub_path).expand();
                 if p.is_relative() {
                    /* We checked that path is ok above so we can do unwrap here */
                    let prefix = path.parent().unwrap();
                    p = prefix.join(p)
                }

                ret.push_str(&pp_helper(&p, level + 1)?);
            } else {
                ret.push_str(l);
                ret.push('\n');
            }
        }

        Ok(ret)
    }

    /// Expands `include` macros in configuration file and other configuration files (eg. themes)
    /// in the filesystem.
    pub fn pp<P: AsRef<Path>>(path: P) -> Result<String> {
        let p_buf: PathBuf = if path.as_ref().is_relative() {
            path.as_ref().expand().canonicalize()?
        } else {
            path.as_ref().expand()
        };

        let mut ret = pp_helper(&p_buf, 0)?;
        drop(p_buf);
        if let Ok(xdg_dirs) = xdg::BaseDirectories::with_prefix("meli") {
            for theme_mailbox in xdg_dirs.find_config_files("themes") {
                let read_dir = std::fs::read_dir(theme_mailbox)?;
                for theme in read_dir {
                    ret.push_str(&pp_helper(&theme?.path(), 0)?);
                }
            }
        }
        Ok(ret)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogSettings {
    #[serde(default)]
    log_file: Option<PathBuf>,
    #[serde(default)]
    maximum_level: melib::LoggingLevel,
}

pub use dotaddressable::*;
mod dotaddressable {
    use super::*;
    pub trait DotAddressable: Serialize {
        fn lookup(&self, parent_field: &str, path: &[&str]) -> Result<String> {
            if !path.is_empty() {
                Err(MeliError::new(format!(
                    "{} has no fields, it is of type {}",
                    parent_field,
                    std::any::type_name::<Self>()
                )))
            } else {
                Ok(toml::to_string(self).map_err(|err| err.to_string())?)
            }
        }
    }

    impl DotAddressable for bool {}

    impl DotAddressable for String {}
    impl DotAddressable for char {}
    impl DotAddressable for IndexStyle {}
    impl DotAddressable for u64 {}
    impl DotAddressable for crate::terminal::Color {}
    impl DotAddressable for crate::terminal::Attr {}
    impl DotAddressable for crate::terminal::Key {}
    impl DotAddressable for usize {}
    impl DotAddressable for Query {}
    impl DotAddressable for melib::LoggingLevel {}
    impl DotAddressable for PathBuf {}
    impl DotAddressable for ToggleFlag {}
    impl DotAddressable for SearchBackend {}
    impl DotAddressable for melib::SpecialUsageMailbox {}
    impl<T: DotAddressable> DotAddressable for Option<T> {}
    impl<T: DotAddressable> DotAddressable for Vec<T> {}
    impl<K: DotAddressable + std::cmp::Eq + std::hash::Hash, V: DotAddressable> DotAddressable
        for HashMap<K, V>
    {
    }
    impl<K: DotAddressable + std::cmp::Eq + std::hash::Hash, V: DotAddressable> DotAddressable
        for IndexMap<K, V>
    {
    }
    impl<K: DotAddressable + std::cmp::Eq + std::hash::Hash> DotAddressable for HashSet<K> {}

    impl DotAddressable for LogSettings {
        fn lookup(&self, parent_field: &str, path: &[&str]) -> Result<String> {
            match path.first() {
                Some(field) => {
                    let tail = &path[1..];
                    match *field {
                        "log_file" => self.log_file.lookup(field, tail),
                        "maximum_level" => self.maximum_level.lookup(field, tail),

                        other => Err(MeliError::new(format!(
                            "{} has no field named {}",
                            parent_field, other
                        ))),
                    }
                }
                None => Ok(toml::to_string(self).map_err(|err| err.to_string())?),
            }
        }
    }

    impl DotAddressable for Settings {
        fn lookup(&self, parent_field: &str, path: &[&str]) -> Result<String> {
            match path.first() {
                Some(field) => {
                    let tail = &path[1..];
                    match *field {
                        "accounts" => self.accounts.lookup(field, tail),
                        "pager" => self.pager.lookup(field, tail),
                        "listing" => self.listing.lookup(field, tail),
                        "notifications" => self.notifications.lookup(field, tail),
                        "shortcuts" => self.shortcuts.lookup(field, tail),
                        "tags" => Err(MeliError::new("unimplemented")),
                        "composing" => Err(MeliError::new("unimplemented")),
                        "pgp" => Err(MeliError::new("unimplemented")),
                        "terminal" => self.terminal.lookup(field, tail),
                        "log" => self.log.lookup(field, tail),

                        other => Err(MeliError::new(format!(
                            "{} has no field named {}",
                            parent_field, other
                        ))),
                    }
                }
                None => Ok(toml::to_string(self).map_err(|err| err.to_string())?),
            }
        }
    }

    impl DotAddressable for AccountConf {
        fn lookup(&self, parent_field: &str, path: &[&str]) -> Result<String> {
            match path.first() {
                Some(field) => {
                    let tail = &path[1..];
                    match *field {
                        "account" => self.account.lookup(field, tail),
                        "conf" => self.conf.lookup(field, tail),
                        "conf_override" => self.conf_override.lookup(field, tail),
                        "mailbox_confs" => self.mailbox_confs.lookup(field, tail),
                        other => Err(MeliError::new(format!(
                            "{} has no field named {}",
                            parent_field, other
                        ))),
                    }
                }
                None => Ok(toml::to_string(self).map_err(|err| err.to_string())?),
            }
        }
    }

    impl DotAddressable for MailUIConf {
        fn lookup(&self, parent_field: &str, path: &[&str]) -> Result<String> {
            match path.first() {
                Some(field) => {
                    let _tail = &path[1..];
                    match *field {
                        "pager" => Err(MeliError::new("unimplemented")), //self.pager.lookup(field, tail),
                        "listing" => Err(MeliError::new("unimplemented")), // self.listing.lookup(field, tail),
                        "notifications" => Err(MeliError::new("unimplemented")), // self.notifications.lookup(field, tail),
                        "shortcuts" => Err(MeliError::new("unimplemented")), //self.shortcuts.lookup(field, tail),
                        "composing" => Err(MeliError::new("unimplemented")), //self.composing.lookup(field, tail),
                        "identity" => Err(MeliError::new("unimplemented")), //self.identity.lookup(field, tail)<String>,
                        "tags" => Err(MeliError::new("unimplemented")), //self.tags.lookup(field, tail),
                        "themes" => Err(MeliError::new("unimplemented")), //self.themes.lookup(field, tail)<Themes>,
                        "pgp" => Err(MeliError::new("unimplemented")), //self.pgp.lookup(field, tail),

                        other => Err(MeliError::new(format!(
                            "{} has no field named {}",
                            parent_field, other
                        ))),
                    }
                }
                None => Ok(toml::to_string(self).map_err(|err| err.to_string())?),
            }
        }
    }

    impl DotAddressable for FileMailboxConf {
        fn lookup(&self, parent_field: &str, path: &[&str]) -> Result<String> {
            match path.first() {
                Some(field) => {
                    let tail = &path[1..];
                    match *field {
                        "conf_override" => self.conf_override.lookup(field, tail),
                        "mailbox_conf" => self.mailbox_conf.lookup(field, tail),
                        other => Err(MeliError::new(format!(
                            "{} has no field named {}",
                            parent_field, other
                        ))),
                    }
                }
                None => Ok(toml::to_string(self).map_err(|err| err.to_string())?),
            }
        }
    }

    impl DotAddressable for FileAccount {
        fn lookup(&self, parent_field: &str, path: &[&str]) -> Result<String> {
            match path.first() {
                Some(field) => {
                    let tail = &path[1..];
                    match *field {
                        "root_mailbox" => self.root_mailbox.lookup(field, tail),
                        "format" => self.format.lookup(field, tail),
                        "identity" => self.identity.lookup(field, tail),
                        "display_name" => self.display_name.lookup(field, tail),
                        "read_only" => self.read_only.lookup(field, tail),
                        "subscribed_mailboxes" => self.subscribed_mailboxes.lookup(field, tail),
                        "mailboxes" => self.mailboxes.lookup(field, tail),
                        "search_backend" => self.search_backend.lookup(field, tail),
                        "manual_refresh" => self.manual_refresh.lookup(field, tail),
                        "refresh_command" => self.refresh_command.lookup(field, tail),
                        "conf_override" => self.conf_override.lookup(field, tail),
                        "extra" => self.extra.lookup(field, tail),
                        other => Err(MeliError::new(format!(
                            "{} has no field named {}",
                            parent_field, other
                        ))),
                    }
                }
                None => Ok(toml::to_string(self).map_err(|err| err.to_string())?),
            }
        }
    }

    impl DotAddressable for melib::AccountSettings {
        fn lookup(&self, parent_field: &str, path: &[&str]) -> Result<String> {
            match path.first() {
                Some(field) => {
                    let tail = &path[1..];
                    match *field {
                        "name" => self.name.lookup(field, tail),
                        "root_mailbox" => self.root_mailbox.lookup(field, tail),
                        "format" => self.format.lookup(field, tail),
                        "identity" => self.identity.lookup(field, tail),
                        "read_only" => self.read_only.lookup(field, tail),
                        "display_name" => self.display_name.lookup(field, tail),
                        "subscribed_mailboxes" => self.subscribed_mailboxes.lookup(field, tail),
                        "mailboxes" => self.mailboxes.lookup(field, tail),
                        "manual_refresh" => self.manual_refresh.lookup(field, tail),
                        "extra" => self.extra.lookup(field, tail),
                        other => Err(MeliError::new(format!(
                            "{} has no field named {}",
                            parent_field, other
                        ))),
                    }
                }
                None => Ok(toml::to_string(self).map_err(|err| err.to_string())?),
            }
        }
    }

    impl DotAddressable for melib::MailboxConf {
        fn lookup(&self, parent_field: &str, path: &[&str]) -> Result<String> {
            match path.first() {
                Some(field) => {
                    let tail = &path[1..];
                    match *field {
                        "alias" => self.alias.lookup(field, tail),
                        "autoload" => self.autoload.lookup(field, tail),
                        "subscribe" => self.subscribe.lookup(field, tail),
                        "ignore" => self.ignore.lookup(field, tail),
                        "usage" => self.usage.lookup(field, tail),
                        "extra" => self.extra.lookup(field, tail),
                        other => Err(MeliError::new(format!(
                            "{} has no field named {}",
                            parent_field, other
                        ))),
                    }
                }
                None => Ok(toml::to_string(self).map_err(|err| err.to_string())?),
            }
        }
    }
}

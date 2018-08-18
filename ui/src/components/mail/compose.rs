/*
 * meli - ui crate
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

pub struct Composer {
    dirty: bool,
    mode: ViewMode,
    pager: Pager,
    buffer: String,
}

impl Default for Composer {
    fn default() -> Self {
        Composer {
            dirty: true,
            mode: ViewMode::Overview,
            pager: Pager::from_str("asdfs\nfdsfds\ndsfdsfs\n\n\n\naaaaaaaaaaaaaa\nfdgfd", None),
            buffer: String::new(),
        }
    }
}

enum ViewMode {
    //Compose,
    Overview,
}

impl fmt::Display for Composer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO display subject/info
        write!(f, "compose")
    }
}

impl Component for Composer {
    fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if self.dirty {
            clear_area(grid, area);
        }
        if !self.buffer.is_empty() {
            eprintln!("{:?}", EnvelopeWrapper::new(self.buffer.as_bytes().to_vec()));

        }
        let upper_left = upper_left!(area);
        let bottom_right = bottom_right!(area);

        let header_height = 12;
        let width = width!(area);
        let mid = if width > 80 {
            let width = width - 80;
            let mid = width / 2;;

            if self.dirty {
                for i in get_y(upper_left)..=get_y(bottom_right) {
                    grid[(mid, i)].set_ch(VERT_BOUNDARY);
                    grid[(mid, i)].set_fg(Color::Default);
                    grid[(mid, i)].set_bg(Color::Default);
                    grid[(mid + 80, i)].set_ch(VERT_BOUNDARY);
                    grid[(mid + 80, i)].set_fg(Color::Default);
                    grid[(mid + 80, i)].set_bg(Color::Default);
                }
            }
            mid
        } else { 0 };

        if self.dirty {
            for i in get_x(upper_left)+ mid + 1..=get_x(upper_left) + mid + 79 {
                grid[(i, header_height)].set_ch(HORZ_BOUNDARY);
                grid[(i, header_height)].set_fg(Color::Default);
                grid[(i, header_height)].set_bg(Color::Default);
            }
        }

        let body_area = ((mid + 1, header_height+2), (mid + 78, get_y(bottom_right)));

        if self.dirty {
            context.dirty_areas.push_back(area);
            self.dirty = false;
        }
        match self.mode {
            ViewMode::Overview => {
                self.pager.draw(grid, body_area, context);

            },
        }
    }

    fn process_event(&mut self, event: &UIEvent, context: &mut Context) {
        match event.event_type {
            UIEventType::Resize => {
                self.dirty = true;
            }
            UIEventType::Input(Key::Char('\n')) => {
                use std::process::{Command, Stdio};
                /* Kill input thread so that spawned command can be sole receiver of stdin */
                {
                    context.input_kill();
                }
                let mut f = if self.buffer.is_empty() {
                    create_temp_file(&new_draft(context), None)
                } else {
                    create_temp_file(&self.buffer.as_bytes(), None)
                };
                //let mut f = Box::new(std::fs::File::create(&dir).unwrap());

                // TODO: check exit status
                Command::new("vim")
                    .arg("+/^$")
                    .arg(&f.path())
                    .stdin(Stdio::inherit())
                    .stdout(Stdio::inherit())
                    .output()
                    .expect("failed to execute process");
                let result = f.read_to_string();
                self.buffer = result.clone();
                self.pager.update_from_string(result);
                context.restore_input();
                self.dirty = true;
                return;
            },
            _ => {},
        }
        self.pager.process_event(event, context);
    }

    fn is_dirty(&self) -> bool {
        self.dirty || self.pager.is_dirty()
    }
    fn set_dirty(&mut self) {
        self.dirty = true;
        self.pager.set_dirty();
    }
}
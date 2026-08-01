//! The behaviour sim, a line-for-line port of the python Pet class: the
//! same states, the same weights, the same gravity constant. Where the
//! python reads ambiguously the python WINS — this file is not the place
//! to redesign a cat.

use rand::Rng;

use crate::sprites;

pub const SCALE: f32 = 4.0;
pub const STRIP_H: f32 = 40.0 * SCALE;
pub const GROUND: f32 = STRIP_H;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum State {
    Walk,
    Sit,
    Sleep,
    Love,
    Follow,
    Carried,
    Falling,
}

pub struct Species {
    pub walk: (&'static [&'static str], &'static [&'static str]),
    pub sit: (&'static [&'static str], &'static [&'static str]),
    pub sleep: (&'static [&'static str], &'static [&'static str]),
    pub ink: sprites::Ink,
}

pub fn cat() -> Species {
    Species {
        walk: (sprites::CAT_WALK_A, sprites::CAT_WALK_B),
        sit: (sprites::CAT_SIT_A, sprites::CAT_SIT_B),
        sleep: (sprites::CAT_SLEEP, sprites::CAT_SLEEP),
        ink: sprites::cat_ink,
    }
}

pub fn clawd() -> Species {
    Species {
        walk: (sprites::CLAWD_WALK_A, sprites::CLAWD_WALK_B),
        sit: (sprites::CLAWD_WALK_A, sprites::CLAWD_WALK_A),
        sleep: (sprites::CLAWD_SLEEP, sprites::CLAWD_SLEEP),
        ink: sprites::clawd_ink,
    }
}

pub struct Pet {
    pub species: Species,
    pub width: f32, // strip width, updated when the surface configures
    pub speed: f32,
    pub span: f32,
    pub x: f32,
    pub dir: f32,
    pub state: State,
    pub frame_no: usize,
    pub ticks_left: i32,
    pub zzz: usize,
    pub follow_x: Option<f32>,
    pub y_off: f32, // height above the ground (carried / falling)
    pub vy: f32,
    tick_n: u64,
}

impl Pet {
    pub fn new(species: Species, width: f32) -> Pet {
        let mut rng = rand::thread_rng();
        let span = species.walk.0.iter().map(|r| r.len()).max().unwrap_or(0) as f32 * SCALE;
        Pet {
            species,
            width,
            speed: 1.0,
            span,
            x: rng.gen_range(50.0..(width - 150.0).max(51.0)),
            dir: if rng.gen_bool(0.5) { 1.0 } else { -1.0 },
            state: State::Walk,
            frame_no: 0,
            ticks_left: rng.gen_range(30..=90),
            zzz: 0,
            follow_x: None,
            y_off: 0.0,
            vy: 0.0,
            tick_n: 0,
        }
    }

    pub fn frame(&self) -> &'static [&'static str] {
        // a state without art must NEVER crash the draw loop (that is how
        // "drag and the pet vanishes" happened in the python)
        let pair = match self.state {
            State::Walk | State::Follow | State::Love | State::Falling => self.species.walk,
            State::Sit | State::Carried => self.species.sit,
            State::Sleep => self.species.sleep,
        };
        // love/carried/falling pin one frame in the python tables where the
        // pair repeats the same art; the walk pair alternates
        if self.frame_no == 0 {
            pair.0
        } else {
            pair.1
        }
    }

    pub fn sprite_h(&self) -> f32 {
        self.frame().len() as f32 * SCALE
    }

    pub fn top_y(&self) -> f32 {
        GROUND - self.sprite_h() - SCALE - self.y_off
    }

    /// (x, top, w, h) — includes the z/♥ space above, python bbox verbatim.
    pub fn bbox(&self) -> (f32, f32, f32, f32) {
        let top = (self.top_y() - 28.0).max(0.0);
        (
            self.x - 8.0,
            top,
            self.span + 16.0,
            (GROUND - top).min(STRIP_H),
        )
    }

    pub fn hit(&self, px: f32, py: f32) -> bool {
        if !(self.x - 8.0 <= px && px <= self.x + self.span + 8.0) {
            return false;
        }
        let (_x, top, _w, h) = self.bbox();
        top <= py && py <= top + h
    }

    pub fn love(&mut self) {
        if self.y_off > 0.0 {
            self.state = State::Falling; // petted mid-air → gravity first
            self.vy = 0.0;
            return;
        }
        self.state = State::Love;
        self.zzz = 0;
        self.ticks_left = 14;
    }

    pub fn toggle_sleep(&mut self) {
        let mut rng = rand::thread_rng();
        if self.y_off > 0.0 {
            self.state = State::Falling;
            self.vy = 0.0;
            return;
        }
        if self.state == State::Sleep {
            self.state = State::Walk;
            self.ticks_left = rng.gen_range(30..=120);
        } else {
            self.state = State::Sleep;
            self.zzz = 0;
            self.ticks_left = rng.gen_range(60..=150);
        }
    }

    pub fn carry(&mut self, px: f32, py: f32) {
        self.state = State::Carried;
        self.x = (px - self.span / 2.0).clamp(0.0, self.width - self.span);
        let want = GROUND - py - self.sprite_h() / 2.0;
        self.y_off = want.clamp(0.0, STRIP_H - self.sprite_h() - SCALE);
    }

    pub fn drop(&mut self) {
        let mut rng = rand::thread_rng();
        if self.y_off > 0.0 {
            self.state = State::Falling; // gravity takes it from here
            self.vy = 0.0;
        } else {
            self.state = State::Walk;
            self.dir = if rng.gen_bool(0.5) { 1.0 } else { -1.0 };
            self.ticks_left = rng.gen_range(30..=120);
        }
    }

    /// One 140 ms tick. Returns whether anything visible changed.
    pub fn tick(&mut self) -> bool {
        let mut rng = rand::thread_rng();
        self.tick_n += 1;
        if self.state == State::Carried {
            return false; // position driven by the pointer
        }
        if self.state == State::Sleep {
            if self.tick_n % 4 != 0 {
                return false;
            }
            self.zzz = (self.zzz + 1) % 3;
        }
        self.frame_no ^= 1;
        self.ticks_left -= 1;
        match self.state {
            State::Walk => {
                self.x += self.dir * SCALE * self.speed;
                if self.x < 10.0 {
                    self.x = 10.0;
                    self.dir = 1.0;
                } else if self.x > self.width - self.span - 10.0 {
                    self.x = self.width - self.span - 10.0;
                    self.dir = -1.0;
                }
            }
            State::Follow => {
                if let Some(fx) = self.follow_x {
                    let fx = fx.clamp(self.span / 2.0, self.width - self.span / 2.0);
                    self.follow_x = Some(fx);
                    let dx = fx - (self.x + self.span / 2.0);
                    if dx.abs() < 24.0 {
                        self.state = State::Sit;
                        self.ticks_left = rng.gen_range(20..=60);
                    } else {
                        self.dir = if dx > 0.0 { 1.0 } else { -1.0 };
                        self.x = (self.x + self.dir * SCALE * 2.0)
                            .clamp(0.0, self.width - self.span);
                    }
                }
            }
            State::Falling => {
                self.vy += 6.0;
                self.y_off -= self.vy;
                if self.y_off <= 0.0 {
                    self.y_off = 0.0;
                    self.state = State::Walk;
                    self.dir = if rng.gen_bool(0.5) { 1.0 } else { -1.0 };
                    self.ticks_left = rng.gen_range(30..=120);
                }
                return true;
            }
            State::Love => {
                self.zzz = (self.zzz + 1) % 3;
            }
            _ => {}
        }
        if self.ticks_left <= 0 {
            self.choose_state();
        }
        true
    }

    fn choose_state(&mut self) {
        let mut rng = rand::thread_rng();
        // python weights: walk 50, sit 25, sleep 12, follow 13
        let roll = rng.gen_range(0..100);
        self.state = if roll < 50 {
            State::Walk
        } else if roll < 75 {
            State::Sit
        } else if roll < 87 {
            State::Sleep
        } else {
            State::Follow
        };
        match self.state {
            State::Walk => {
                self.dir = if rng.gen_bool(0.5) { 1.0 } else { -1.0 };
                self.ticks_left = rng.gen_range(30..=120);
            }
            State::Sit => self.ticks_left = rng.gen_range(20..=60),
            State::Follow => {
                self.follow_x = None;
                self.ticks_left = rng.gen_range(40..=90);
            }
            _ => self.ticks_left = rng.gen_range(60..=150),
        }
    }
}

mod agent;
mod app;
mod music;
mod spaces;
mod ui;

use app::State;
use zellij_tile::prelude::*;

register_plugin!(State);

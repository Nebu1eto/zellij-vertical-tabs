mod agent;
mod app;
mod ui;

use app::State;
use zellij_tile::prelude::*;

register_plugin!(State);

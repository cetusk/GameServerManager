//! Valheim catalog, legacy configuration and read-only world inspection.
//! Runtime operations are supplied through the shared local backend.
pub mod config;
pub mod editor;
pub mod launch;
pub mod local;
pub mod world;
use gsm_domain::{GameDescriptor, GameId};

pub fn descriptor() -> GameDescriptor {
    GameDescriptor {
        id: GameId::try_from("valheim".to_owned()).expect("static game ID"),
        name: "Valheim",
        description: "ワールド、modifier、接続先",
        sample_world: "Meadows",
        backup_scope: "ワールド全体と管理設定",
    }
}

pub mod settings;

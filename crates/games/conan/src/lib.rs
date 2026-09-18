//! Game-owned catalog and local runtime plan.
pub mod local;
use gsm_domain::{GameDescriptor, GameId};

pub fn descriptor() -> GameDescriptor {
    GameDescriptor {
        id: GameId::try_from("conan".to_owned()).expect("static game ID"),
        name: "Conan Exiles",
        description: "SQLite、設定カタログ、MOD",
        sample_world: "Exiled Lands",
        backup_scope: "game.db、設定、MOD リスト",
    }
}

pub mod settings;

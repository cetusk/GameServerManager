//! Game-owned catalog and local runtime plan.
pub mod local;
use gsm_domain::{GameDescriptor, GameId};

pub fn descriptor() -> GameDescriptor {
    GameDescriptor {
        id: GameId::try_from("arksa".to_owned()).expect("static game ID"),
        name: "ARK: Survival Ascended",
        description: "マップ、RCON、プロファイル",
        sample_world: "The Island",
        backup_scope: "選択マップの SavedArks と設定",
    }
}

pub mod settings;

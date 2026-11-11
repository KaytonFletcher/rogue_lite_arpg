use avian2d::prelude::*;
use bevy::prelude::*;

use super::attack::{on_start_melee, on_stop_melee};
use super::swing::MeleeSwingKind;
use super::{MELEE_WEAPON_REACH, MeleeWeapon};
use crate::prelude::*;

pub fn sword(sprites: &SpriteAssets) -> impl Scene {
    bsn! {
        @melee_weapon(sprites.sword.clone(), 120, WeaponFamily::Sword)
        Name::new("Sword")
        ~{MeleeWeapon {
            damage: Damage::Range { min: 1.0, max: 6.0 },
            hitbox: Collider::rectangle(10.0, 40.0),
            swing_kind: MeleeSwingKind::STAB,
            attack_time: WeaponFamily::Sword.attack_seconds(),
            hold_distance: 15.0,
        }}
        Knockback(10.0)
    }
}

pub fn axe(sprites: &SpriteAssets) -> impl Scene {
    bsn! {
        @melee_weapon(sprites.axe.clone(), 220, WeaponFamily::Axe)
        Name::new("Axe")
        ~{axe_swing()}
        Knockback(20.0)
    }
}

pub fn freeze_axe(sprites: &SpriteAssets) -> impl Scene {
    bsn! {
        @melee_weapon(sprites.axe.clone(), 220, WeaponFamily::Axe)
        Name::new("Freeze Axe")
        ~{axe_swing()}
        Knockback(2.0)
        Effects [
            Frozen
            Lifespan::new(2.0)
        ]
    }
}

/// The heavy slash both axes share.
fn axe_swing() -> MeleeWeapon {
    MeleeWeapon {
        damage: Damage::Range {
            min: 2.0,
            max: 12.0,
        },
        hitbox: Collider::rectangle(10.0, 40.0),
        swing_kind: MeleeSwingKind::SLASH,
        attack_time: WeaponFamily::Axe.attack_seconds(),
        hold_distance: 30.0,
    }
}

/// Seconds between the end of one swing and the earliest next one. The cooldown starts with the
/// swing, so it is the swing's length plus this.
const MELEE_RECOVERY: f32 = 0.05;

/// What every melee weapon shares: a mainhand item worth `value`, drawn with `image`, that
/// swings like the rest of its `family` when used.
fn melee_weapon(image: Handle<Image>, value: u32, family: WeaponFamily) -> impl Scene {
    let cooldown = family.attack_seconds() + MELEE_RECOVERY;
    bsn! {
        family
        ~Equippable::new(EquipmentSlot::Mainhand, cooldown, &MELEE_EQUIPMENT_POSES)
            .with_reach(MELEE_WEAPON_REACH)
        ~Item::new(value, ItemKind::Melee)
        Sprite { image }
        on(on_stop_melee)
        on(on_start_melee)
    }
}

static MELEE_EQUIPMENT_POSES: DirectionalPoses = DirectionalPoses {
    up: EquipmentPose::above(0.0, -8.0, 30.0),
    down: EquipmentPose::behind(0.0, 8.0, -30.0),
    left: EquipmentPose::behind(-8.0, -15.0, 90.0),
    right: EquipmentPose::above(8.0, -15.0, -90.0),
};

use bevy::prelude::*;

use crate::prelude::*;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (
            sync_held_weapon,
            apply_weapon_sheet,
            update_animation_state,
            cycle_character_animation,
        )
            .chain()
            .in_set(VfxSystems::Select),
    );
}

/// The [`WeaponFamily`] of the character's mainhand item, or `None` when the hand is empty or
/// holds something outside the families. Frozen once the character is dying, so dropping the
/// weapon on death doesn't swap sheets mid-fall.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Deref)]
pub(super) struct HeldWeapon(Option<WeaponFamily>);

/// Sheets for a character whose art draws the weapon in hand: one per weapon family it can hold,
/// plus `unarmed` for an empty hand. While a family with a sheet is held, its sheet is shown and
/// the item's own sprite is hidden so the weapon isn't drawn twice. A family without a sheet
/// falls back to `unarmed` with the item sprite drawn on top, as before.
#[derive(Component, Clone, Debug)]
pub(in crate::character) struct WeaponSheets {
    pub unarmed: Handle<Image>,
    pub sword: Option<Handle<Image>>,
    pub axe: Option<Handle<Image>>,
    pub staff: Option<Handle<Image>>,
}

impl WeaponSheets {
    /// The sheet that draws `family` in hand, if there is one.
    fn drawing(&self, family: WeaponFamily) -> Option<&Handle<Image>> {
        match family {
            WeaponFamily::Sword => self.sword.as_ref(),
            WeaponFamily::Axe => self.axe.as_ref(),
            WeaponFamily::Staff => self.staff.as_ref(),
        }
    }
}

fn sync_held_weapon(
    mut holders: Query<(&mut HeldWeapon, Option<&Mainhand>, &CharacterAnimationState)>,
    families: Query<&WeaponFamily>,
) {
    for (mut held, mainhand, state) in &mut holders {
        if *state == CharacterAnimationState::Dying {
            continue;
        }
        let family = mainhand.and_then(|hand| families.get(hand.get()).ok().copied());
        held.set_if_neq(HeldWeapon(family));
    }
}

fn apply_weapon_sheet(
    mut holders: Query<(&HeldWeapon, &WeaponSheets, &mut Sprite, Option<&Mainhand>)>,
    mut items: Query<&mut Visibility, With<Equipped>>,
) {
    for (held, sheets, mut sprite, mainhand) in &mut holders {
        let drawn = held.0.and_then(|family| sheets.drawing(family));
        let image = drawn.unwrap_or(&sheets.unarmed);
        if sprite.image != *image {
            sprite.image = image.clone();
        }
        // Equipping makes the item visible, so this runs every frame rather than on change.
        if drawn.is_some()
            && let Some(mainhand) = mainhand
            && let Ok(mut visibility) = items.get_mut(mainhand.get())
        {
            visibility.set_if_neq(Visibility::Hidden);
        }
    }
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[require(FacingDirection, Kinematics)]
pub(super) enum CharacterAnimationState {
    #[default]
    Idle,
    Moving,
    Attacking,
    Dying,
}

impl CharacterAnimationState {
    /// Row group index in [`AnimationSheet::animations`].
    fn index(self) -> usize {
        match self {
            Self::Idle => 0,
            Self::Moving => 1,
            Self::Attacking => 2,
            Self::Dying => 3,
        }
    }
}

/// Sheet layout: columns plus one animation per state and facing direction.
#[derive(Component, Clone, Debug)]
pub(super) struct AnimationSheet {
    columns: usize,
    animations: [[AnimationData; 4]; 4],
    preserve_phase: bool,
}

impl Default for AnimationSheet {
    fn default() -> Self {
        // Rows are ordered Down, Right, Up, Left, matching the sheet.
        Self {
            columns: 13,
            animations: [
                state_rows([14, 15, 12, 13], 3, 0.5),
                // Column 0 of a walk row is the standing pose; the loop is columns 1-8.
                // Including it puts a stand-still hitch in every stride.
                state_rows_from([10, 11, 8, 9], 1, 8, 0.1),
                state_rows([18, 19, 16, 17], 9, 0.1),
                state_rows([20, 20, 20, 20], 5, 0.4),
            ],
            preserve_phase: false,
        }
    }
}

impl AnimationSheet {
    /// The knight's sheets (`tools/sprites/lpc_sheet.py` builds them): the LPC layout with a
    /// 4-frame breathing idle and a 7-frame death that buckles, slumps and topples.
    pub(super) fn knight() -> Self {
        let mut sheet = Self::default();
        sheet.animations[CharacterAnimationState::Idle.index()] =
            state_rows([14, 15, 12, 13], 4, 0.3);
        sheet.animations[CharacterAnimationState::Dying.index()] =
            state_rows([20, 20, 20, 20], 7, 0.22);
        sheet
    }

    /// Flapping flier layout sharing one wingbeat loop.
    pub(super) fn bat() -> Self {
        const BAT_FRAMES_PER_ROW: usize = 4;
        const BAT_FLAP_DURATION: f32 = 0.12;
        const BAT_ROWS: [usize; 4] = [0, 1, 2, 3];

        Self {
            columns: BAT_FRAMES_PER_ROW,
            animations: [
                state_rows(BAT_ROWS, BAT_FRAMES_PER_ROW, BAT_FLAP_DURATION),
                state_rows(BAT_ROWS, BAT_FRAMES_PER_ROW, BAT_FLAP_DURATION),
                state_rows(BAT_ROWS, BAT_FRAMES_PER_ROW, BAT_FLAP_DURATION),
                state_rows([0, 0, 0, 0], 1, BAT_FLAP_DURATION),
            ],
            preserve_phase: true,
        }
    }

    fn get(&self, state: CharacterAnimationState, direction: FacingDirection) -> &AnimationData {
        &self.animations[state.index()][direction_index(direction)]
    }
}

fn state_rows(rows: [usize; 4], frame_count: usize, frame_duration: f32) -> [AnimationData; 4] {
    state_rows_from(rows, 0, frame_count, frame_duration)
}

fn state_rows_from(
    rows: [usize; 4],
    first_column: usize,
    frame_count: usize,
    frame_duration: f32,
) -> [AnimationData; 4] {
    rows.map(|row| AnimationData {
        row,
        first_column,
        frame_count,
        frame_duration,
    })
}

fn direction_index(direction: FacingDirection) -> usize {
    match direction {
        FacingDirection::Down => 0,
        FacingDirection::Right => 1,
        FacingDirection::Up => 2,
        FacingDirection::Left => 3,
    }
}

fn update_animation_state(
    mut character_query: Query<
        (
            &Kinematics,
            &Attacking,
            Option<&Health>,
            &mut CharacterAnimationState,
            &mut FacingDirection,
        ),
        Or<(Changed<Kinematics>, Changed<Attacking>, Changed<Health>)>,
    >,
) {
    for (kinematics, attacking, health, mut animation_state, mut facing_direction) in
        &mut character_query
    {
        if health.is_none_or(|h| h.hp <= 0.0) {
            animation_state.set_if_neq(CharacterAnimationState::Dying);
            continue;
        }

        // Attacking animation takes priority over walking / idle. Facing stays locked for the
        // swing: a facing change re-selects the animation and would restart it mid-swing.
        if **attacking {
            animation_state.set_if_neq(CharacterAnimationState::Attacking);
            continue;
        }

        facing_direction.set_if_neq(facing_direction.turned_toward(kinematics.direction));

        if kinematics.is_moving() {
            animation_state.set_if_neq(CharacterAnimationState::Moving);
        } else {
            animation_state.set_if_neq(CharacterAnimationState::Idle);
        }
    }
}

fn cycle_character_animation(
    mut commands: Commands,
    mut query: Query<
        (
            Entity,
            &mut AnimationIndices,
            Option<&mut AnimationTimer>,
            &mut Sprite,
            &CharacterAnimationState,
            &FacingDirection,
            &AnimationSheet,
            Option<&HeldWeapon>,
        ),
        Or<(Changed<CharacterAnimationState>, Changed<FacingDirection>)>,
    >,
) {
    for (entity, mut indices, timer, mut sprite, state, direction, sheet, held) in &mut query {
        let mut animation = *sheet.get(*state, *direction);
        // An attack lasts as long as the weapon's use, so the swing on screen ends with the
        // hitbox (or the cast with the bolt) instead of being cut off or running past it.
        if *state == CharacterAnimationState::Attacking
            && let Some(family) = held.and_then(|held| held.0)
        {
            let frames = animation.frame_count as f32;
            animation.frame_duration = family.attack_seconds() / frames;
        }
        let first = animation.row * sheet.columns + animation.first_column;
        let last = first + animation.frame_count - 1;

        // Row-major atlas: carry the frame column across a row change so a
        // shared loop keeps its beat instead of restarting on every turn.
        let offset = if sheet.preserve_phase {
            sprite
                .texture_atlas
                .as_ref()
                .map_or(0, |atlas| atlas.index % sheet.columns)
                % animation.frame_count
        } else {
            0
        };

        // A phase-preserving sheet also keeps its clock, or a facing flip
        // every frame would freeze the loop. Only a duration change re-arms it.
        // A finished one-shot (attack, death) has had its timer removed by `animate_sprites`.
        let frame_duration = std::time::Duration::from_secs_f32(animation.frame_duration);
        let fresh = AnimationTimer(Timer::from_seconds(
            animation.frame_duration,
            TimerMode::Repeating,
        ));
        match timer {
            Some(mut timer) if !sheet.preserve_phase || timer.duration() != frame_duration => {
                *timer = fresh;
            }
            Some(_) => {}
            None => {
                commands.entity(entity).insert(fresh);
            }
        }

        if let Some(atlas) = &mut sprite.texture_atlas {
            atlas.index = first + offset;
        }

        // Attacks and dying play once and hold their last frame. A looping death would flash the
        // character back to standing just as the despawn / game-over timer ends, and a looping
        // attack would restart the swing if the weapon's use ran a moment long.
        // `animate_sprites` drops the timer once the one-shot runs out.
        if matches!(
            state,
            CharacterAnimationState::Attacking | CharacterAnimationState::Dying
        ) {
            *indices = AnimationIndices::OneShot(first + 1..=last);
            continue;
        }

        // `Cycle::next` yields the range start first, so leave the iterator one
        // step ahead of the displayed frame.
        let mut cycle = (first..=last).cycle();
        for _ in 0..=offset {
            cycle.next();
        }
        *indices = AnimationIndices::Cycle(cycle);
    }
}

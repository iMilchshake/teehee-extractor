//! Feature definitions and filtering for extracted data.

use std::collections::HashSet;

/// All available features with their group assignments
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature {
    // position group
    PosX,
    PosY,
    // velocity group
    VelX,
    VelY,
    // cursor group (raw target position)
    CursorX,
    CursorY,
    // aim group (derived from cursor)
    AimAngle,
    AimDistance,
    // movement group
    MoveDir,
    // inputs group
    KeyJump,
    KeyFire,
    KeyHook,
    // state group
    IsGrounded,
    FreezeStatus,
    // hook group
    HookGrabbed,
    HookPosX,
    HookPosY,
    // weapons group
    IsHammer,
    IsGun,
    IsOtherWeapon,
    // jumps group
    CanJump,
}

impl Feature {
    /// Get the string name of this feature (used in CLI and HDF5 metadata)
    pub fn name(&self) -> &'static str {
        match self {
            Feature::PosX => "pos_x",
            Feature::PosY => "pos_y",
            Feature::VelX => "vel_x",
            Feature::VelY => "vel_y",
            Feature::CursorX => "cursor_x",
            Feature::CursorY => "cursor_y",
            Feature::AimAngle => "aim_angle",
            Feature::AimDistance => "aim_distance",
            Feature::MoveDir => "move_dir",
            Feature::KeyJump => "key_jump",
            Feature::KeyFire => "key_fire",
            Feature::KeyHook => "key_hook",
            Feature::IsGrounded => "is_grounded",
            Feature::FreezeStatus => "freeze_status",
            Feature::HookGrabbed => "hook_grabbed",
            Feature::HookPosX => "hook_pos_x",
            Feature::HookPosY => "hook_pos_y",
            Feature::IsHammer => "is_hammer",
            Feature::IsGun => "is_gun",
            Feature::IsOtherWeapon => "is_other_weapon",
            Feature::CanJump => "can_jump",
        }
    }

    /// Get the group this feature belongs to
    pub fn group(&self) -> &'static str {
        match self {
            Feature::PosX | Feature::PosY => "position",
            Feature::VelX | Feature::VelY => "velocity",
            Feature::CursorX | Feature::CursorY => "cursor",
            Feature::AimAngle | Feature::AimDistance => "aim",
            Feature::MoveDir => "movement",
            Feature::KeyJump | Feature::KeyFire | Feature::KeyHook => "inputs",
            Feature::IsGrounded | Feature::FreezeStatus => "state",
            Feature::HookGrabbed | Feature::HookPosX | Feature::HookPosY => "hook",
            Feature::IsHammer | Feature::IsGun | Feature::IsOtherWeapon => "weapons",
            Feature::CanJump => "jumps",
        }
    }

    /// All features in canonical order
    pub fn all() -> &'static [Feature] {
        &[
            Feature::PosX,
            Feature::PosY,
            Feature::VelX,
            Feature::VelY,
            Feature::CursorX,
            Feature::CursorY,
            Feature::AimAngle,
            Feature::AimDistance,
            Feature::MoveDir,
            Feature::KeyJump,
            Feature::KeyFire,
            Feature::KeyHook,
            Feature::IsGrounded,
            Feature::FreezeStatus,
            Feature::HookGrabbed,
            Feature::HookPosX,
            Feature::HookPosY,
            Feature::IsHammer,
            Feature::IsGun,
            Feature::IsOtherWeapon,
            Feature::CanJump,
        ]
    }

    /// All available group names
    pub fn groups() -> &'static [&'static str] {
        &[
            "position", "velocity", "cursor", "aim", "movement", "inputs", "state", "hook",
            "weapons", "jumps",
        ]
    }

    /// Get all features in a group
    pub fn in_group(group: &str) -> Vec<Feature> {
        Feature::all()
            .iter()
            .filter(|f| f.group() == group)
            .copied()
            .collect()
    }

    /// Parse a feature or group name into features
    pub fn parse(name: &str) -> Option<Vec<Feature>> {
        // check if it's a group name
        if Feature::groups().contains(&name) {
            return Some(Feature::in_group(name));
        }
        // check if it's a feature name
        Feature::all()
            .iter()
            .find(|f| f.name() == name)
            .map(|f| vec![*f])
    }
}

/// Selected features for extraction
#[derive(Debug, Clone)]
pub struct FeatureSet {
    features: Vec<Feature>,
}

impl Default for FeatureSet {
    fn default() -> Self {
        Self::all()
    }
}

impl FeatureSet {
    /// Create a feature set with all features
    pub fn all() -> Self {
        Self {
            features: Feature::all().to_vec(),
        }
    }

    /// Create a feature set from a comma-separated string of feature/group names
    pub fn from_spec(spec: &str) -> Result<Self, String> {
        let mut features = Vec::new();
        let mut seen = HashSet::new();

        for name in spec.split(',').map(|s| s.trim()) {
            if name.is_empty() {
                continue;
            }
            match Feature::parse(name) {
                Some(parsed) => {
                    for f in parsed {
                        if seen.insert(f) {
                            features.push(f);
                        }
                    }
                }
                None => {
                    return Err(format!(
                        "Unknown feature or group: '{}'. Available groups: {:?}, features: {:?}",
                        name,
                        Feature::groups(),
                        Feature::all().iter().map(|f| f.name()).collect::<Vec<_>>()
                    ));
                }
            }
        }

        if features.is_empty() {
            return Err("No features selected".to_string());
        }

        Ok(Self { features })
    }

    /// Get the selected features in order
    pub fn features(&self) -> &[Feature] {
        &self.features
    }

    /// Get feature names in order
    pub fn names(&self) -> Vec<&'static str> {
        self.features.iter().map(|f| f.name()).collect()
    }

    /// Number of selected features
    pub fn len(&self) -> usize {
        self.features.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.features.is_empty()
    }

    /// Extract a single feature value from TickData
    pub fn extract_single(&self, tick: &crate::TickData, feature: Feature) -> f32 {
        match feature {
            Feature::PosX => tick.pos_x,
            Feature::PosY => tick.pos_y,
            Feature::VelX => tick.vel_x,
            Feature::VelY => tick.vel_y,
            Feature::CursorX => tick.cursor_x,
            Feature::CursorY => tick.cursor_y,
            Feature::AimAngle => tick.aim_angle,
            Feature::AimDistance => tick.aim_distance,
            Feature::MoveDir => tick.move_dir,
            Feature::KeyJump => tick.key_jump,
            Feature::KeyFire => tick.key_fire,
            Feature::KeyHook => tick.key_hook,
            Feature::IsGrounded => tick.is_grounded,
            Feature::FreezeStatus => tick.freeze_status,
            Feature::HookGrabbed => tick.hook_grabbed,
            Feature::HookPosX => tick.hook_pos_x,
            Feature::HookPosY => tick.hook_pos_y,
            Feature::IsHammer => tick.is_hammer,
            Feature::IsGun => tick.is_gun,
            Feature::IsOtherWeapon => tick.is_other_weapon,
            Feature::CanJump => tick.can_jump,
        }
    }

    /// Extract values from TickData for selected features
    pub fn extract(&self, tick: &crate::TickData) -> Vec<f32> {
        self.features
            .iter()
            .map(|f| match f {
                Feature::PosX => tick.pos_x,
                Feature::PosY => tick.pos_y,
                Feature::VelX => tick.vel_x,
                Feature::VelY => tick.vel_y,
                Feature::CursorX => tick.cursor_x,
                Feature::CursorY => tick.cursor_y,
                Feature::AimAngle => tick.aim_angle,
                Feature::AimDistance => tick.aim_distance,
                Feature::MoveDir => tick.move_dir,
                Feature::KeyJump => tick.key_jump,
                Feature::KeyFire => tick.key_fire,
                Feature::KeyHook => tick.key_hook,
                Feature::IsGrounded => tick.is_grounded,
                Feature::FreezeStatus => tick.freeze_status,
                Feature::HookGrabbed => tick.hook_grabbed,
                Feature::HookPosX => tick.hook_pos_x,
                Feature::HookPosY => tick.hook_pos_y,
                Feature::IsHammer => tick.is_hammer,
                Feature::IsGun => tick.is_gun,
                Feature::IsOtherWeapon => tick.is_other_weapon,
                Feature::CanJump => tick.can_jump,
            })
            .collect()
    }
}

//! What the panel needs from a host: the 72 parameters of the shell spike (8 macros, 64 slots),
//! as normalised values with begin/set/end gestures. The plugin implements [`HostParams`] over
//! nice-plug's `ParamSetter`; tests and the dev window use [`LocalParams`].

use crate::layout;

pub const NUM_MACROS: usize = 8;
pub const NUM_SLOTS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamId {
    Macro(usize),
    Slot(usize),
}

pub trait HostParams {
    /// Normalised (0..1) value.
    fn get(&self, id: ParamId) -> f32;
    /// A gesture starts (mouse down on a knob).
    fn begin(&mut self, id: ParamId);
    fn set(&mut self, id: ParamId, normalized: f32);
    /// The gesture ends.
    fn end(&mut self, id: ParamId);
}

/// In-memory parameters with the layout's defaults.
pub struct LocalParams {
    pub macros: [f32; NUM_MACROS],
    pub slots: [f32; NUM_SLOTS],
    /// Open gestures (begin without end); tests check they balance.
    pub open_gestures: i32,
}

impl Default for LocalParams {
    fn default() -> Self {
        Self {
            macros: layout::MACROS.map(|m| m.default),
            slots: std::array::from_fn(layout::slot_default),
            open_gestures: 0,
        }
    }
}

impl HostParams for LocalParams {
    fn get(&self, id: ParamId) -> f32 {
        match id {
            ParamId::Macro(i) => self.macros[i],
            ParamId::Slot(i) => self.slots[i],
        }
    }

    fn begin(&mut self, _id: ParamId) {
        self.open_gestures += 1;
    }

    fn set(&mut self, id: ParamId, normalized: f32) {
        match id {
            ParamId::Macro(i) => self.macros[i] = normalized,
            ParamId::Slot(i) => self.slots[i] = normalized,
        }
    }

    fn end(&mut self, _id: ParamId) {
        self.open_gestures -= 1;
    }
}

//! `params!` and `enum_items!`: declare a `const` table and its index enum from one list, so their order cannot
//! drift, and check the table at compile time.

/// Declare a parameter table and its index enum from one list, checked at compile time.
///
/// ```
/// use enth_params::{params, ParamSpec, Scale, Unit};
///
/// params! {
///     /// Parameters of a toy filter, in spec order.
///     pub enum ToyParam in TOY_PARAMS {
///         Cutoff = ParamSpec::float("cutoff", "Cutoff", Unit::Hz, 20.0, 20_000.0, Scale::Log, 2_000.0)
///             .modulatable()
///             .basic("Brightness: where the filter starts to cut."),
///         Resonance = ParamSpec::float("resonance", "Resonance", Unit::Percent, 0.0, 100.0, Scale::Linear, 10.0)
///             .basic("Emphasis at the cutoff."),
///     }
/// }
///
/// assert_eq!(ToyParam::COUNT, 2);
/// assert_eq!(ToyParam::Resonance.index(), 1);
/// assert_eq!(ToyParam::Cutoff.spec().key, "cutoff");
/// assert_eq!(ToyParam::from_key("resonance"), Some(ToyParam::Resonance));
/// assert_eq!(TOY_PARAMS[1].default, 10.0);
/// ```
///
/// It emits:
///
/// - the enum, `#[repr(u8)]`, with `ALL`, `COUNT`, `index()`, `spec()`, `key()` and `from_key()`;
/// - `static TABLE: [ParamSpec; COUNT]`, in the same order;
/// - a `const` item that runs [`check_table`](crate::check_table): every spec is valid, keys are unique, and each
///   variant name spells its key (`PhaseMode` ↔ `phase_mode`). A violation is a compile error (`E0080`) whose
///   message names the table and the key.
///
/// A default out of range does not compile:
///
/// ```compile_fail,E0080
/// use enth_params::{params, ParamSpec, Scale, Unit};
/// params! {
///     pub enum Bad in BAD {
///         Cutoff = ParamSpec::float("cutoff", "Cutoff", Unit::Hz, 20.0, 20_000.0, Scale::Log, 10.0)
///             .basic("10 Hz is below the 20 Hz minimum."),
///     }
/// }
/// ```
///
/// Nor does a Log scale that starts at zero, a variant that does not spell its key, a duplicate key, a structural
/// parameter that is also modulatable, or a spec without a description:
///
/// ```compile_fail,E0080
/// use enth_params::{params, ParamSpec, Scale, Unit};
/// params! {
///     pub enum Bad in BAD {
///         Attack = ParamSpec::float("attack", "Attack", Unit::Ms, 0.0, 10_000.0, Scale::Log, 5.0).basic("x"),
///     }
/// }
/// ```
///
/// ```compile_fail,E0080
/// use enth_params::{params, ParamSpec, Scale, Unit};
/// params! {
///     pub enum Bad in BAD {
///         Cutoff = ParamSpec::float("resonance", "Resonance", Unit::Percent, 0.0, 100.0, Scale::Linear, 10.0).basic("x"),
///     }
/// }
/// ```
///
/// ```compile_fail,E0080
/// use enth_params::{params, ParamSpec, Scale, Unit};
/// params! {
///     pub enum Bad in BAD {
///         Level = ParamSpec::float("level", "Level", Unit::Db, -60.0, 6.0, Scale::Linear, 0.0).basic("x"),
///         Level2 = ParamSpec::float("level", "Level", Unit::Db, -60.0, 6.0, Scale::Linear, 0.0).basic("x"),
///     }
/// }
/// ```
///
/// ```compile_fail,E0080
/// use enth_params::{params, ParamSpec, Scale, Unit};
/// params! {
///     pub enum Bad in BAD {
///         Rate = ParamSpec::float("rate", "Rate", Unit::Hz, 0.01, 50.0, Scale::Log, 2.0).structural().modulatable().basic("x"),
///     }
/// }
/// ```
///
/// ```compile_fail,E0080
/// use enth_params::{params, ParamSpec, Scale, Unit};
/// params! {
///     pub enum Bad in BAD {
///         Level = ParamSpec::float("level", "Level", Unit::Db, -60.0, 6.0, Scale::Linear, 0.0),
///     }
/// }
/// ```
#[macro_export]
macro_rules! params {
    (
        $(#[$meta:meta])*
        $vis:vis enum $Enum:ident in $TABLE:ident {
            $( $(#[$vmeta:meta])* $Variant:ident = $spec:expr ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u8)]
        $vis enum $Enum {
            $( $(#[$vmeta])* $Variant ),+
        }

        impl $Enum {
            /// Every parameter, in spec order (the order of values in a module instance).
            pub const ALL: &'static [$Enum] = &[ $( $Enum::$Variant ),+ ];
            /// The number of parameters.
            pub const COUNT: usize = Self::ALL.len();

            /// The position of this parameter in the table and in a module's dense value list.
            #[inline]
            pub const fn index(self) -> usize {
                self as usize
            }

            /// The spec of this parameter.
            #[inline]
            pub const fn spec(self) -> &'static $crate::ParamSpec {
                &$TABLE[self as usize]
            }

            /// The key of this parameter (`"cutoff"`).
            #[inline]
            pub const fn key(self) -> &'static str {
                $TABLE[self as usize].key
            }

            /// The parameter with this key (exact match).
            pub fn from_key(key: &str) -> ::core::option::Option<Self> {
                Self::ALL.iter().copied().find(|p| p.key() == key)
            }
        }

        $vis static $TABLE: [$crate::ParamSpec; $Enum::COUNT] = [ $( $spec ),+ ];

        const _: () = $crate::check_table(stringify!($TABLE), &$TABLE, &[ $( stringify!($Variant) ),+ ]);
    };
}

/// Declare the items of an enum parameter and a matching Rust enum from one list, checked at compile time.
///
/// ```
/// use enth_params::enum_items;
///
/// enum_items! {
///     /// Filter responses.
///     pub enum Mode in MODES {
///         Lp = ("lp", "Low-pass", "Keeps the lows."),
///         Hp = ("hp", "High-pass", "Keeps the highs."),
///     }
/// }
///
/// assert_eq!(MODES[1].key, "hp");
/// assert_eq!(Mode::from_value(1.0), Some(Mode::Hp));
/// assert_eq!(Mode::from_value(0.5), None);
/// assert_eq!(Mode::Lp.key(), "lp");
/// ```
///
/// It emits the enum (`#[repr(u8)]`, with `ALL`, `COUNT`, `index()`, `key()`, `from_index()` and `from_value()`,
/// which decodes a stored `f32` without panicking), `static TABLE: [EnumItem; COUNT]`, and a `const` check that
/// keys are valid and unique and that each variant spells its key.
#[macro_export]
macro_rules! enum_items {
    (
        $(#[$meta:meta])*
        $vis:vis enum $Enum:ident in $TABLE:ident {
            $( $(#[$vmeta:meta])* $Variant:ident = ($key:expr, $name:expr, $desc:expr) ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u8)]
        $vis enum $Enum {
            $( $(#[$vmeta])* $Variant ),+
        }

        impl $Enum {
            /// Every item, in index order.
            pub const ALL: &'static [$Enum] = &[ $( $Enum::$Variant ),+ ];
            /// The number of items.
            pub const COUNT: usize = Self::ALL.len();

            /// The item's index: the value stored in a patch.
            #[inline]
            pub const fn index(self) -> usize {
                self as usize
            }

            /// The item's key (`"saw"`).
            #[inline]
            pub const fn key(self) -> &'static str {
                $TABLE[self as usize].key
            }

            /// The item with this index.
            #[inline]
            pub const fn from_index(index: usize) -> ::core::option::Option<Self> {
                if index < Self::COUNT { ::core::option::Option::Some(Self::ALL[index]) } else { ::core::option::Option::None }
            }

            /// The item a stored value names, or `None` if the value is not an exact index. Never panics, so the
            /// audio thread may call it.
            #[inline]
            pub fn from_value(value: f32) -> ::core::option::Option<Self> {
                if value >= 0.0 && value < Self::COUNT as f32 && value.fract() == 0.0 {
                    Self::from_index(value as usize)
                } else {
                    ::core::option::Option::None
                }
            }
        }

        $vis static $TABLE: [$crate::EnumItem; $Enum::COUNT] = [ $( $crate::EnumItem::new($key, $name, $desc) ),+ ];

        const _: () = $crate::check_items(stringify!($TABLE), &$TABLE, &[ $( stringify!($Variant) ),+ ]);
    };
}

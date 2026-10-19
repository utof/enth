//! Notes: the host's note events as engine events with CLAP's note tuple, `NOTE_END` reports, and polyphonic
//! modulation keyed by the host's voice ids (lane SK for the translation; lane PL completes voice info, poly-mod
//! policy and VST3 parity). Allocation-free: runs inside `process()`.
//!
//! - A note's key is `NoteKey { port: 0, channel, key, note_id: VoiceID::id() or −1 }`. Read `VoiceID::id()`, never
//!   `id_or_fallback` (it ignores the host's id). In a note-off or choke a wildcard field is −1 (the engine's
//!   wildcard); a note-on needs a key (a wildcard key is dropped) and takes channel 0 for a wildcard channel.
//! - Host notes are on port 0, so a host note-off never releases an editor audition note
//!   ([`enth_session::AUDITION_PORT`]), and audition notes' `NOTE_END`s are not reported to the host.
//! - `PolyModulation` becomes `EventKind::MacroVoice` for that voice id; `MonoAutomation` is ignored (`process()`
//!   reads the parameters' modulated values directly, so applying it too would count it twice). A `PolyModulation`
//!   for a voice id that no playing note carries changes nothing (the engine matches by note id), and a note-on
//!   starts its voice's offsets at 0, so a host that modulates before the note starts (CLAP allows it) loses that
//!   offset until it sends the next one.
//! - MIDI CC 1 (the mod wheel's MSB), on any channel, becomes `EventKind::ModWheel` with nice-plug's value
//!   (`cc / 127`). nice-plug delivers CCs only with `MidiConfig::MidiCCs` (docs/plugin.md, the `MidiCCs` decision);
//!   other CCs, CC 33 (the wheel's LSB), pitch bend and pressure are ignored: the engine has no input for them.

use enth_engine::{Event, EventKind, NoteEnd, NoteKey};
use enth_patch::model::MACROS;
use enth_session::AUDITION_PORT;
use nice_plug::midi::{Channel, Key, NoteEvent, VoiceID, control_change};

/// The port of every host note (CLAP has one note input port).
pub const HOST_PORT: i16 = 0;

fn channel_of(channel: Channel) -> i16 {
    channel.number().map_or(-1, i16::from)
}

fn key_of(key: Key) -> i16 {
    key.number().map_or(-1, i16::from)
}

fn id_of(voice_id: VoiceID) -> i32 {
    voice_id.id().unwrap_or(-1)
}

/// The engine event for a host note event, or `None` for the kinds the engine does not take.
pub fn engine_event<S>(event: &NoteEvent<S>) -> Option<Event> {
    let (offset, kind) = match *event {
        NoteEvent::NoteOn {
            timing,
            voice_id,
            channel,
            key,
            velocity,
        } => {
            let key = key.number()?;
            (
                timing,
                EventKind::NoteOn {
                    key: NoteKey {
                        port: HOST_PORT,
                        channel: channel.number().map_or(0, i16::from),
                        key: i16::from(key),
                        note_id: id_of(voice_id),
                    },
                    velocity,
                },
            )
        }
        NoteEvent::NoteOff {
            timing,
            voice_id,
            channel,
            key,
            velocity,
        } => (
            timing,
            EventKind::NoteOff {
                key: NoteKey {
                    port: HOST_PORT,
                    channel: channel_of(channel),
                    key: key_of(key),
                    note_id: id_of(voice_id),
                },
                velocity,
            },
        ),
        NoteEvent::Choke {
            timing,
            voice_id,
            channel,
            key,
        } => (
            timing,
            EventKind::Choke {
                key: NoteKey {
                    port: HOST_PORT,
                    channel: channel_of(channel),
                    key: key_of(key),
                    note_id: id_of(voice_id),
                },
            },
        ),
        NoteEvent::PolyModulation {
            timing,
            voice_id,
            poly_modulation_id,
            normalized_offset,
        } => {
            let index = u8::try_from(poly_modulation_id)
                .ok()
                .filter(|&i| usize::from(i) < MACROS)?;
            (
                timing,
                EventKind::MacroVoice {
                    key: NoteKey::note_id(voice_id),
                    index,
                    offset: normalized_offset,
                },
            )
        }
        NoteEvent::MidiCC {
            timing,
            cc: control_change::MODULATION_MSB,
            value,
            ..
        } => (timing, EventKind::ModWheel { value }),
        _ => return None,
    };
    Some(Event { offset, kind })
}

/// The host's `NOTE_END` for a voice that ended: the note's own tuple (as the host gave it), at `timing`. `None`
/// for an editor audition note.
pub fn note_end<S>(key: NoteKey, timing: u32) -> Option<NoteEvent<S>> {
    if key.port == AUDITION_PORT {
        return None;
    }
    Some(NoteEvent::VoiceTerminated {
        timing,
        voice_id: if key.note_id >= 0 {
            VoiceID::ID(key.note_id)
        } else {
            VoiceID::Wildcard
        },
        channel: u8::try_from(key.channel).map_or(Channel::Wildcard, Channel::Number),
        key: u8::try_from(key.key).map_or(Key::Wildcard, Key::Number),
    })
}

/// [`note_end`] for an engine report, at its offset inside the block (`NoteEnd::offset`, already clamped).
pub fn reported_end<S>(end: &NoteEnd) -> Option<NoteEvent<S>> {
    note_end(end.key, end.offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    type Ev = NoteEvent<()>;

    #[test]
    fn note_ons_carry_the_hosts_voice_id_never_a_fallback() {
        let on: Ev = NoteEvent::NoteOn {
            timing: 17,
            voice_id: VoiceID::ID(1234),
            channel: Channel::Number(2),
            key: Key::Number(60),
            velocity: 0.5,
        };
        let e = engine_event(&on).unwrap();
        assert_eq!(e.offset, 17);
        assert_eq!(
            e.kind,
            EventKind::NoteOn {
                key: NoteKey {
                    port: 0,
                    channel: 2,
                    key: 60,
                    note_id: 1234
                },
                velocity: 0.5
            }
        );
        let anonymous: Ev = NoteEvent::NoteOn {
            timing: 0,
            voice_id: VoiceID::Wildcard,
            channel: Channel::Wildcard,
            key: Key::Number(61),
            velocity: 1.0,
        };
        let EventKind::NoteOn { key, .. } = engine_event(&anonymous).unwrap().kind else {
            panic!()
        };
        assert_eq!((key.channel, key.note_id), (0, -1));
        let keyless: Ev = NoteEvent::NoteOn {
            timing: 0,
            voice_id: VoiceID::ID(1),
            channel: Channel::Number(0),
            key: Key::Wildcard,
            velocity: 1.0,
        };
        assert_eq!(engine_event(&keyless), None);
    }

    #[test]
    fn note_offs_keep_wildcards_on_the_host_port() {
        let off: Ev = NoteEvent::NoteOff {
            timing: 3,
            voice_id: VoiceID::Wildcard,
            channel: Channel::Wildcard,
            key: Key::Wildcard,
            velocity: 0.0,
        };
        let EventKind::NoteOff { key, .. } = engine_event(&off).unwrap().kind else {
            panic!()
        };
        assert_eq!(
            key,
            NoteKey {
                port: 0,
                channel: -1,
                key: -1,
                note_id: -1
            }
        );
        // Port 0, not the wildcard: a host note-off never releases an audition note.
        let audition = NoteKey {
            port: AUDITION_PORT,
            channel: 0,
            key: 60,
            note_id: 0,
        };
        assert!(!key.matches(&audition));
        let choke: Ev = NoteEvent::Choke {
            timing: 0,
            voice_id: VoiceID::ID(9),
            channel: Channel::Number(1),
            key: Key::Number(64),
        };
        assert!(matches!(
            engine_event(&choke).unwrap().kind,
            EventKind::Choke { .. }
        ));
    }

    #[test]
    fn poly_modulation_addresses_the_voice_id_and_mono_automation_is_ignored() {
        let poly: Ev = NoteEvent::PolyModulation {
            timing: 5,
            voice_id: 77,
            poly_modulation_id: 2,
            normalized_offset: -0.25,
        };
        assert_eq!(
            engine_event(&poly).unwrap().kind,
            EventKind::MacroVoice {
                key: NoteKey::note_id(77),
                index: 2,
                offset: -0.25
            }
        );
        let other: Ev = NoteEvent::PolyModulation {
            timing: 5,
            voice_id: 77,
            poly_modulation_id: 8,
            normalized_offset: 0.5,
        };
        assert_eq!(engine_event(&other), None);
        let mono: Ev = NoteEvent::MonoAutomation {
            timing: 0,
            poly_modulation_id: 0,
            normalized_value: 0.5,
        };
        assert_eq!(engine_event(&mono), None);
    }

    #[test]
    fn note_ends_report_the_hosts_tuple_and_skip_auditions() {
        let end: Ev = note_end(
            NoteKey {
                port: 0,
                channel: 3,
                key: 62,
                note_id: 41,
            },
            12,
        )
        .unwrap();
        assert_eq!(
            end,
            NoteEvent::VoiceTerminated {
                timing: 12,
                voice_id: VoiceID::ID(41),
                channel: Channel::Number(3),
                key: Key::Number(62)
            }
        );
        let anonymous: Ev = note_end(
            NoteKey {
                port: 0,
                channel: 0,
                key: 62,
                note_id: -1,
            },
            0,
        )
        .unwrap();
        assert!(matches!(
            anonymous,
            NoteEvent::VoiceTerminated {
                voice_id: VoiceID::Wildcard,
                ..
            }
        ));
        let audition = NoteKey {
            port: AUDITION_PORT,
            channel: 0,
            key: 60,
            note_id: 3,
        };
        assert_eq!(note_end::<()>(audition, 0), None);
    }
}

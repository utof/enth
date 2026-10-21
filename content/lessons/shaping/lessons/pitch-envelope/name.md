# Pitch envelope

Routed to an oscillator's [Semitones](enth:osc1.semi), env2 bends the pitch at each note: up by the
[route](enth:env2->osc1.semi)'s depth, then back down as env2 decays.

- **Depth:** how far above the note it starts; 12.5 % is an octave.
- env2's [Decay](enth:env2.decay): how long the drop takes. A few tens of milliseconds make a kick; longer makes a
  zap.

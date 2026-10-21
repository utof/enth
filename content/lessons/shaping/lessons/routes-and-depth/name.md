# Routes and depth

A **route** connects a modulator to a knob. Here **env2**, an envelope that shapes nothing on its own, moves the
[cutoff](enth:filter1.cutoff): [the route](enth:env2->filter1.cutoff) lifts it at each note, and it falls back as
env2 decays.

- **Depth** is how far the route pushes: on the cutoff, 10 % is about an octave.
- The **cutoff knob** is where the sweep ends.
- env2's [Decay](enth:env2.decay) is the time to fall from full to the sustain level: how long the sweep lasts.

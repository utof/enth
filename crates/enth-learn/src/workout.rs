//! **Owner: lane DRL** (Drill player, freeze point F1e). Pre-declared by lane S0e (R4/R5, freeze point F0e); empty until
//! DRL writes it. The reference is docs/ear.md, "Workouts".
//!
//! The pure **workout composer**: 3 games of 12 to 20 items, about 10 to 15 minutes, chosen by due skills, ending in
//! "done for today" with "one more" optional. **Deterministic:** the same progress file on the same day gives the same
//! workout, and the day comes from the shell. Due skills choose the games (a due `svf.cutoff` comes back as Spot the
//! change one day, Dial it in another and Frequency finder later: varied retrieval), each category is met alone before
//! confusable ones mix, and after a break the warm-up starts one level below the last level reached with no "you missed
//! N days". Progress is thresholds per skill in physical units and a cumulative calendar of practised days. Not here, by
//! design: streak counters, loss messages, daily quests, currency, leaderboards, perfect-pitch promises.

//! S2a's reserved kinds (lane S0d, freeze F0m) stay out of generator v1: its start holds none, and no target at any
//! level or musical setting adds one, so no draw can name one (docs/phase-s2.md, rule 6: the generator never lists a
//! reserved kind). Generator v1 never adds or removes a module, so this holds until a generator version that does.

use enth_challenge::random::{self, GENERATOR_V1, TEMPLATE_STARTER};
use enth_challenge::{Category, RandomSpec};
use enth_patch::ModuleKind;

fn hidden_modules(p: &enth_patch::Patch) -> Vec<String> {
    p.modules()
        .filter(|(_, m)| m.kind.is_hidden())
        .map(|(_, m)| format!("{} ({})", m.id, m.kind))
        .collect()
}

#[test]
fn generator_v1_never_draws_a_reserved_kind() {
    assert!(ModuleKind::ALL.iter().any(|k| k.is_hidden()));
    let start = random::start(TEMPLATE_STARTER).unwrap();
    assert_eq!(hidden_modules(&start), Vec::<String>::new());
    for seed in 0..300u64 {
        let spec = RandomSpec {
            generator: GENERATOR_V1,
            seed,
            difficulty: [0, 25, 50, 75, 100][seed as usize % 5],
            amounts: [None; Category::COUNT],
            musical: [100, 0][seed as usize % 2],
            template: TEMPLATE_STARTER,
        };
        let g = random::target(&spec).unwrap();
        assert_eq!(
            hidden_modules(&g.patch),
            Vec::<String>::new(),
            "seed {seed}"
        );
        for addr in &g.params {
            if let Some(id) = addr.module_id() {
                let (_, m) = g.patch.find(id).unwrap();
                assert!(!m.kind.is_hidden(), "seed {seed}: {addr}");
            }
        }
    }
}

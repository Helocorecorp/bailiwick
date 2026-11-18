import Bailiwick

/-! Which axioms each headline theorem rests on. CI fails if any of them
depends on `sorryAx` (an unproven step) or on anything beyond Lean's three
standard axioms: `propext`, `Classical.choice`, `Quot.sound`. -/

open Bailiwick

#print axioms contains_inter
#print axioms minTrust_mono
#print axioms feasible_iff
#print axioms feasible_effective
#print axioms policies_only_narrow
#print axioms policy_order_irrelevant
#print axioms policy_reverse_irrelevant
#print axioms dependencies_only_raise
#print axioms deploys_has_feasible
#print axioms deploys_meets_redundancy
#print axioms no_feasible_no_deploy
#print axioms no_compelled_plaintext
#print axioms customer_keys_enforced
#print axioms custody_only_helps
#print axioms latency_met
#print axioms unmeasured_excluded
#print axioms no_users_no_latency_rule
#print axioms adopting_changes_nothing_iff

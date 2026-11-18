import Bailiwick.Model

/-!
# What is true of Bailiwick

Each theorem here is checked by Lean's kernel. A change to the model that
breaks one of them does not build.
-/

namespace Bailiwick

/-! ## Trust -/

/-- More sensitive data never needs less trust. -/
theorem minTrust_mono {a b : Nat} (h : a ≤ b) : minTrust a ≤ minTrust b := by
  match a, b with
  | 0, 0 | 0, 1 | 0, 2 | 1, 1 | 1, 2 | 2, 2 => decide
  | 0, _ + 3 | 1, _ + 3 | 2, _ + 3 => simp [minTrust]
  | _ + 3, _ + 3 => simp [minTrust]
  | 1, 0 | 2, 0 | 2, 1 | _ + 3, 0 | _ + 3, 1 | _ + 3, 2 => omega

/-- Every sensitivity needs at least trust 1 and at most trust 3. -/
theorem minTrust_bounds (s : Nat) : 1 ≤ minTrust s ∧ minTrust s ≤ 3 := by
  match s with
  | 0 | 1 | 2 => decide
  | _ + 3 => simp [minTrust]

/-! ## Residency is a set algebra -/

theorem mem_normalize_country (s : List Loc) (c : String) :
    Loc.country c ∈ normalize s ↔ Loc.country c ∈ s := by
  simp [normalize, List.mem_filter]

theorem mem_normalize_sub (s : List Loc) (c d : String) :
    Loc.sub c d ∈ normalize s ↔ Loc.sub c d ∈ s ∧ Loc.country c ∉ s := by
  simp [normalize, List.mem_filter]

/-- **Intersection is exact.** An offer is inside the intersection of two
residencies if and only if it is inside both — including the sub-national
cases, where a country covers its subdivisions and a subdivision-only
residency rejects an offer whose subdivision is unknown. -/
theorem contains_inter (r₁ r₂ : Residency) (c : String) (sub : Option String) :
    (r₁.inter r₂).contains c sub = (r₁.contains c sub && r₂.contains c sub) := by
  cases r₁ with
  | global => simp [Residency.inter, Residency.contains]
  | set a =>
    cases r₂ with
    | global => simp [Residency.inter, Residency.contains]
    | set b =>
      simp only [Residency.inter, Residency.contains]
      cases sub with
      | none =>
        simp [mem_normalize_country, List.mem_append, List.mem_filter, covers]
        intro hb ha
        exact ⟨ha, hb⟩
      | some d =>
        simp only [Bool.decide_or, Bool.decide_eq_true]
        rw [Bool.eq_iff_iff]
        simp [mem_normalize_country, mem_normalize_sub, List.mem_append, List.mem_filter, covers]
        by_cases ca : Loc.country c ∈ a <;> by_cases cb : Loc.country c ∈ b <;>
          by_cases sa : Loc.sub c d ∈ a <;> by_cases sb : Loc.sub c d ∈ b <;>
          simp [ca, cb, sa, sb]

/-- `global` admits everything. -/
theorem contains_global (c : String) (sub : Option String) :
    Residency.global.contains c sub = true := rfl

/-- Folding constraints is intersection: `none` is "unconstrained". -/
theorem contains_foldRes (a b : Option Residency) (c : String) (sub : Option String) :
    (Residency.ofOption (foldRes a b)).contains c sub =
      ((Residency.ofOption a).contains c sub && (Residency.ofOption b).contains c sub) := by
  cases a <;> cases b <;> simp [foldRes, Residency.ofOption, contains_inter, contains_global]

/-! ## Every exclusion is a failed rule, and only a failed rule excludes -/

theorem isEmpty_eq_true {α} (l : List α) : l.isEmpty = true ↔ l = [] := by
  cases l <;> simp [List.isEmpty]

theorem append_eq_nil' {α} (a b : List α) : a ++ b = [] ↔ a = [] ∧ b = [] := by
  cases a <;> simp

theorem filterMap_eq_nil' {α β} (f : α → Option β) (l : List α) :
    l.filterMap f = [] ↔ ∀ x ∈ l, f x = none := by
  induction l with
  | nil => simp
  | cons x xs ih =>
    simp only [List.filterMap_cons, List.mem_cons, forall_eq_or_imp]
    cases f x <;> simp [ih]

variable {Q : Type} [LE Q] [DecidableRel (α := Q) (· ≤ ·)]

/-- What the first seven rules ask of an offer, stated directly. -/
def basePass (u : Work Q) (eff : Effective) (o : Offer Q) : Prop :=
  o.classes.contains u.cls = true ∧
  (∃ c, o.country = some c ∧ (u.effResidency eff).contains c o.sub = true ∧
    deniedBy eff.denyCountries c = none) ∧
  (∃ t, o.trust = some t ∧ u.needTrust eff ≤ t) ∧
  (∀ v, o.vendor = some v → deniedBy eff.denyVendors v = none) ∧
  (∀ n need, (n, need) ∈ u.needs → ∃ amount, lookupRes o.resources n = some amount ∧ need ≤ amount) ∧
  (∀ f ∈ u.features, f ∈ o.features)

/-- What rule 10 asks of an offer: if the unit has a latency target, then
from every region its users are in, a measurement within it. -/
def latencyPass (u : Work Q) (o : Offer Q) : Prop :=
  ∀ t, u.slo = some t → ∀ r ∈ u.sloFrom, ∃ h, lookupRes o.latency r = some h ∧ h ≤ t

theorem latencyExcl_nil (u : Work Q) (o : Offer Q) : latencyExcl u o = [] ↔ latencyPass u o := by
  unfold latencyExcl latencyPass
  cases hs : u.slo with
  | none => simp
  | some t =>
    simp only [filterMap_eq_nil', Option.some.injEq, forall_eq']
    constructor
    · intro h r hr
      have := h r hr
      cases hl : lookupRes o.latency r with
      | none => simp [hl] at this
      | some a =>
        simp only [hl] at this
        refine ⟨a, rfl, ?_⟩
        by_cases hle : a ≤ t
        · exact hle
        · simp [hle] at this
    · intro h r hr
      obtain ⟨a, hl, hle⟩ := h r hr
      simp [hl, hle]

/-- What all ten rules ask of an offer: the first seven; no denied
jurisdiction can compel plaintext from it (8); if the unit needs
customer-held keys, the customer holds them (9); and its measured latency
meets the unit's target from where the users are (10). -/
def rulesPass (u : Work Q) (eff : Effective) (o : Offer Q) : Prop :=
  basePass u eff o ∧
  (∀ y ∈ eff.denyJurisdictions, o.exposedTo y.1 = false) ∧
  (u.needCustomerKeys eff = true → o.customerKeys = true) ∧
  latencyPass u o

/-- **An offer is excluded exactly when one of the ten rules fails.** No
offer is rejected for any other reason, and none that fails a rule gets
through. -/
theorem feasible_iff (u : Work Q) (eff : Effective) (o : Offer Q) :
    feasible u eff o ↔ rulesPass u eff o := by
  unfold feasible rulesPass basePass evalOffer
  simp only [isEmpty_eq_true, List.append_assoc, append_eq_nil', filterMap_eq_nil', and_assoc]
  apply and_congr
  · split <;> simp_all
  apply and_congr
  · cases o.country with
    | none => simp
    | some c =>
      simp only [Option.some.injEq, exists_eq_left', append_eq_nil']
      constructor
      · intro ⟨h1, h2⟩
        refine ⟨?_, ?_⟩
        · split at h1 <;> simp_all
        · split at h2 <;> simp_all
      · intro ⟨h1, h2⟩
        refine ⟨?_, ?_⟩
        · simp [h1]
        · simp [h2]
  apply and_congr
  · cases o.trust with
    | none => simp
    | some t =>
      simp only [Option.some.injEq, exists_eq_left']
      by_cases h : t < u.needTrust eff <;> simp [h] <;> omega
  apply and_congr
  · cases o.vendor with
    | none => simp
    | some v =>
      simp only [Option.some.injEq, forall_eq']
      cases deniedBy eff.denyVendors v <;> simp
  apply and_congr
  · constructor
    · intro h n need hm
      have := h (n, need) hm
      cases hl : lookupRes o.resources n with
      | none => simp [hl] at this
      | some amount =>
        simp only [hl] at this
        refine ⟨amount, rfl, ?_⟩
        by_cases hle : need ≤ amount
        · exact hle
        · simp [hle] at this
    · intro h p hm
      obtain ⟨amount, hl, hle⟩ := h p.1 p.2 hm
      simp [hl, hle]
  apply and_congr
  · constructor
    · intro h f hf
      have := h f hf
      by_cases hc : f ∈ o.features
      · exact hc
      · simp [hc] at this
    · intro h f hf
      simp [h f hf]
  apply and_congr
  · constructor
    · intro h y hy
      obtain ⟨j, p⟩ := y
      have := h (j, p) hy
      cases he : o.exposedTo j <;> simp_all
    · intro h y hy
      obtain ⟨j, p⟩ := y
      have := h (j, p) hy
      simp_all
  apply and_congr
  · cases u.needCustomerKeys eff <;> cases o.customerKeys <;> simp
  · exact latencyExcl_nil u o

/-! ## Policies only narrow, and their order does not matter -/

theorem deniedBy_none (ds : List (String × String)) (x : String) :
    deniedBy ds x = none ↔ ∀ y ∈ ds, y.1 ≠ x := by
  induction ds with
  | nil => simp [deniedBy]
  | cons y ys ih =>
    simp only [deniedBy, List.find?] at *
    by_cases h : y.1 = x
    · simp [h]
    · have hb : (y.1 == x) = false := by simp [h]
      simp [hb, ih, h]

theorem addDeny_clear (acc : List (String × String)) (pol : String) (items : List String) (x : String) :
    (∀ y ∈ addDeny acc pol items, y.1 ≠ x) ↔ (∀ y ∈ acc, y.1 ≠ x) ∧ x ∉ items := by
  induction items generalizing acc with
  | nil => simp [addDeny]
  | cons i is ih =>
    simp only [addDeny, List.foldl] at *
    rw [ih]
    by_cases ha : acc.any (·.1 == i) = true
    · simp only [ha, if_true, List.mem_cons, not_or]
      constructor
      · rintro ⟨hacc, hnot⟩
        refine ⟨hacc, ?_, hnot⟩
        intro hix
        obtain ⟨y, hy, hyi⟩ := List.any_eq_true.mp ha
        exact hacc y hy (by simp at hyi; rw [hyi, hix])
      · rintro ⟨hacc, _, hnot⟩
        exact ⟨hacc, hnot⟩
    · simp only [ha, if_false, Bool.false_eq_true, List.mem_append, List.mem_cons,
        List.not_mem_nil, or_false, not_or]
      constructor
      · rintro ⟨hall, hnot⟩
        exact ⟨fun y hy => hall y (Or.inl hy), fun h => hall (i, pol) (Or.inr rfl) h.symm, hnot⟩
      · rintro ⟨hacc, hix, hnot⟩
        refine ⟨?_, hnot⟩
        rintro y (hy | rfl)
        · exact hacc y hy
        · exact fun h => hix h.symm

theorem fold_residency (ps : List Policy) (e : Effective) (c : String) (sub : Option String) :
    (Residency.ofOption (ps.foldl Effective.add e).residency).contains c sub =
      ((Residency.ofOption e.residency).contains c sub &&
        ps.all (fun p => (Residency.ofOption p.residency).contains c sub)) := by
  induction ps generalizing e with
  | nil => simp
  | cons p ps ih =>
    simp only [List.foldl, ih, Effective.add, contains_foldRes, List.all_cons, Bool.and_assoc]

theorem fold_minTrust (ps : List Policy) (e : Effective) (t : Nat) :
    (ps.foldl Effective.add e).minTrust ≤ t ↔ e.minTrust ≤ t ∧ ∀ p ∈ ps, p.minTrust.getD 0 ≤ t := by
  induction ps generalizing e with
  | nil => simp
  | cons p ps ih =>
    simp only [List.foldl, ih, Effective.add, Nat.max_le, List.mem_cons, forall_eq_or_imp, and_assoc]

theorem fold_denyVendors (ps : List Policy) (e : Effective) (x : String) :
    (∀ y ∈ (ps.foldl Effective.add e).denyVendors, y.1 ≠ x) ↔
      (∀ y ∈ e.denyVendors, y.1 ≠ x) ∧ ∀ p ∈ ps, x ∉ p.denyVendors := by
  induction ps generalizing e with
  | nil => simp
  | cons p ps ih =>
    simp only [List.foldl, ih, Effective.add, addDeny_clear, List.mem_cons, forall_eq_or_imp, and_assoc]

theorem fold_denyCountries (ps : List Policy) (e : Effective) (x : String) :
    (∀ y ∈ (ps.foldl Effective.add e).denyCountries, y.1 ≠ x) ↔
      (∀ y ∈ e.denyCountries, y.1 ≠ x) ∧ ∀ p ∈ ps, x ∉ p.denyCountries := by
  induction ps generalizing e with
  | nil => simp
  | cons p ps ih =>
    simp only [List.foldl, ih, Effective.add, addDeny_clear, List.mem_cons, forall_eq_or_imp, and_assoc]

/-- Denials that do not depend on a fixed item: every denied entry
satisfies `P` exactly when the earlier ones did and every new item does. -/
theorem addDeny_all (P : String → Prop) (acc : List (String × String)) (pol : String)
    (items : List String) :
    (∀ y ∈ addDeny acc pol items, P y.1) ↔ (∀ y ∈ acc, P y.1) ∧ ∀ x ∈ items, P x := by
  induction items generalizing acc with
  | nil => simp [addDeny]
  | cons i is ih =>
    simp only [addDeny, List.foldl] at *
    rw [ih]
    by_cases ha : acc.any (·.1 == i) = true
    · simp only [ha, if_true, List.mem_cons, forall_eq_or_imp]
      constructor
      · rintro ⟨hacc, hrest⟩
        obtain ⟨y, hy, hyi⟩ := List.any_eq_true.mp ha
        have : y.1 = i := by simpa using hyi
        exact ⟨hacc, this ▸ hacc y hy, hrest⟩
      · rintro ⟨hacc, _, hrest⟩
        exact ⟨hacc, hrest⟩
    · simp only [ha, if_false, Bool.false_eq_true, List.mem_append, List.mem_cons,
        List.not_mem_nil, or_false, forall_eq_or_imp]
      constructor
      · rintro ⟨hall, hrest⟩
        exact ⟨fun y hy => hall y (Or.inl hy), hall (i, pol) (Or.inr rfl), hrest⟩
      · rintro ⟨hacc, hi, hrest⟩
        refine ⟨?_, hrest⟩
        rintro y (hy | rfl)
        · exact hacc y hy
        · exact hi

theorem fold_denyJurisdictions (P : String → Prop) (ps : List Policy) (e : Effective) :
    (∀ y ∈ (ps.foldl Effective.add e).denyJurisdictions, P y.1) ↔
      (∀ y ∈ e.denyJurisdictions, P y.1) ∧ ∀ p ∈ ps, ∀ x ∈ p.denyJurisdictions, P x := by
  induction ps generalizing e with
  | nil => simp
  | cons p ps ih =>
    simp only [List.foldl, ih, Effective.add, addDeny_all, List.mem_cons, forall_eq_or_imp, and_assoc]

theorem fold_customerKeys (ps : List Policy) (e : Effective) :
    (ps.foldl Effective.add e).customerKeys = (e.customerKeys || ps.any (·.customerKeys)) := by
  induction ps generalizing e with
  | nil => simp
  | cons p ps ih => simp only [List.foldl, ih, Effective.add, List.any_cons, Bool.or_assoc]

/-- What one policy, on its own, asks of an offer under the first seven
rules. -/
def admitsBase (p : Policy) (o : Offer Q) : Prop :=
  (∀ c, o.country = some c → (Residency.ofOption p.residency).contains c o.sub = true) ∧
  (∀ c, o.country = some c → c ∉ p.denyCountries) ∧
  (∀ t, o.trust = some t → p.minTrust.getD 0 ≤ t) ∧
  (∀ v, o.vendor = some v → v ∉ p.denyVendors)

/-- What one policy, on its own, asks of an offer. -/
def admits (p : Policy) (o : Offer Q) : Prop :=
  admitsBase p o ∧
  (∀ j ∈ p.denyJurisdictions, o.exposedTo j = false) ∧
  (p.customerKeys = true → o.customerKeys = true)

theorem ofOption_none : Residency.ofOption none = .global := rfl

theorem effResidency_contains (u : Work Q) (eff : Effective) (c : String) (sub : Option String) :
    (u.effResidency eff).contains c sub =
      ((u.effResidency Effective.empty).contains c sub && (Residency.ofOption eff.residency).contains c sub) := by
  simp only [Work.effResidency, contains_foldRes, Effective.empty, ofOption_none, contains_global, Bool.and_true]

theorem needTrust_le (u : Work Q) (eff : Effective) (t : Nat) :
    u.needTrust eff ≤ t ↔ u.needTrust Effective.empty ≤ t ∧ eff.minTrust ≤ t := by
  simp [Work.needTrust, Effective.empty, Nat.max_le]

theorem basePass_effective (u : Work Q) (ps : List Policy) (o : Offer Q) :
    basePass u (effective ps) o ↔ basePass u Effective.empty o ∧ ∀ p ∈ ps, admitsBase p o := by
  unfold basePass admitsBase effective
  have hres := fun c sub => fold_residency ps Effective.empty c sub
  simp only [Effective.empty, Residency.ofOption, contains_global, Bool.true_and] at hres
  simp only [effResidency_contains (eff := ps.foldl Effective.add Effective.empty), hres,
    needTrust_le (eff := ps.foldl Effective.add Effective.empty), fold_minTrust, deniedBy_none,
    fold_denyVendors, fold_denyCountries, Bool.and_eq_true, List.all_eq_true]
  simp only [Effective.empty, Residency.ofOption, contains_global, List.not_mem_nil, false_implies,
    implies_true, true_and, Nat.zero_le, and_true, deniedBy, List.find?, Option.map_none']
  cases o.country <;> cases o.trust <;> cases o.vendor <;>
    simp [forall_and] <;> constructor <;> intro h <;> simp_all [forall_and]

theorem needCustomerKeys_eq (u : Work Q) (eff : Effective) :
    u.needCustomerKeys eff = (u.needCustomerKeys Effective.empty || eff.customerKeys) := by
  simp [Work.needCustomerKeys, Effective.empty]

/-- **The policies in force, exactly.** An offer is feasible under any set of
policies if and only if it is feasible with no policy at all *and* every
single policy admits it. Nothing about composing policies can make an offer
feasible that one of them, alone, rules out. -/
theorem feasible_effective (u : Work Q) (ps : List Policy) (o : Offer Q) :
    feasible u (effective ps) o ↔ feasible u Effective.empty o ∧ ∀ p ∈ ps, admits p o := by
  rw [feasible_iff, feasible_iff]
  unfold rulesPass admits
  rw [basePass_effective]
  have hj := fold_denyJurisdictions (fun j => o.exposedTo j = false) ps Effective.empty
  have hk := fold_customerKeys ps Effective.empty
  unfold effective
  rw [hj, needCustomerKeys_eq u (ps.foldl Effective.add Effective.empty), hk]
  simp only [Effective.empty, List.not_mem_nil, false_implies, implies_true, true_and,
    Bool.false_or, Bool.or_false, Bool.or_eq_true, List.any_eq_true, forall_and]
  constructor
  · rintro ⟨⟨hb0, hbp⟩, hjp, hk, hl⟩
    refine ⟨⟨hb0, fun h => hk (Or.inl h), hl⟩, hbp, hjp, fun p hp hpk => hk (Or.inr ⟨p, hp, hpk⟩)⟩
  · rintro ⟨⟨hb0, hk0, hl⟩, hbp, hjp, hkp⟩
    refine ⟨⟨hb0, hbp⟩, hjp, ?_, hl⟩
    rintro (h | ⟨p, hp, hpk⟩)
    · exact hk0 h
    · exact hkp p hp hpk

/-- **Policies only narrow.** Adding policies never makes an offer feasible
that was not feasible before. -/
theorem policies_only_narrow (u : Work Q) (ps qs : List Policy) (o : Offer Q) :
    feasible u (effective (ps ++ qs)) o → feasible u (effective ps) o := by
  rw [feasible_effective, feasible_effective]
  rintro ⟨h0, hall⟩
  exact ⟨h0, fun p hp => hall p (List.mem_append_left qs hp)⟩

/-- **Order does not matter.** Any two lists of the same policies — in any
order, with any repetition — give the same verdicts. -/
theorem policy_order_irrelevant (u : Work Q) {ps qs : List Policy} (hsame : ∀ p, p ∈ ps ↔ p ∈ qs)
    (o : Offer Q) : feasible u (effective ps) o ↔ feasible u (effective qs) o := by
  rw [feasible_effective, feasible_effective]
  exact and_congr Iff.rfl ⟨fun h p hp => h p ((hsame p).mpr hp), fun h p hp => h p ((hsame p).mp hp)⟩

/-- **Adopting a policy changes nothing exactly when everything already
allowed satisfies it.** This is what `bailiwick prove --implies` decides:
it looks for an offer the unit allows that the policy, alone, forbids. -/
theorem adopting_changes_nothing_iff (u : Work Q) (ps : List Policy) (p : Policy) :
    (∀ o : Offer Q, feasible u (effective ps) o → feasible u (effective (ps ++ [p])) o) ↔
      (∀ o : Offer Q, feasible u (effective ps) o → admits p o) := by
  constructor
  · intro h o hf
    exact ((feasible_effective u (ps ++ [p]) o).mp (h o hf)).2 p (List.mem_append_right ps (List.mem_singleton_self p))
  · intro h o hf
    have hp := h o hf
    rw [feasible_effective] at hf ⊢
    refine ⟨hf.1, fun q hq => ?_⟩
    rcases List.mem_append.mp hq with hq | hq
    · exact hf.2 q hq
    · rw [List.mem_singleton] at hq
      subst hq
      exact hp

/-- In particular, reversing the order in which policies are imported
changes nothing. -/
theorem policy_reverse_irrelevant (u : Work Q) (ps : List Policy) (o : Offer Q) :
    feasible u (effective ps.reverse) o ↔ feasible u (effective ps) o :=
  policy_order_irrelevant u (fun _ => List.mem_reverse) o

/-! ## Data dependencies only add requirements -/

theorem fold_deps_residency (ds : List Dep) (r0 : Option Residency) (c : String) (sub : Option String) :
    (Residency.ofOption (ds.foldl (fun acc d => foldRes acc d.residency) r0)).contains c sub =
      ((Residency.ofOption r0).contains c sub &&
        ds.all (fun d => (Residency.ofOption d.residency).contains c sub)) := by
  induction ds generalizing r0 with
  | nil => simp
  | cons d ds ih => simp only [List.foldl, ih, contains_foldRes, List.all_cons, Bool.and_assoc]

theorem fold_deps_sensitivity (ds : List Dep) (s0 : Nat) :
    s0 ≤ ds.foldl (fun s d => max s (d.sensitivity.getD 0)) s0 := by
  induction ds generalizing s0 with
  | nil => exact Nat.le_refl _
  | cons d ds ih => exact Nat.le_trans (Nat.le_max_left _ _) (ih _)

/-- **Dependencies only add requirements.** A service that is feasible with
its data dependencies is feasible without them: depending on regulated data
can tighten where a service may run, never loosen it. -/
theorem dependencies_only_raise (u : Work Q) (eff : Effective) (o : Offer Q) :
    feasible u eff o → feasible { u with deps := [] } eff o := by
  rw [feasible_iff, feasible_iff]
  rintro ⟨⟨hcap, ⟨c, hc, hres, hdc⟩, ⟨t, ht, htrust⟩, hv, hneeds, hfeat⟩, hj, hk, hl⟩
  refine ⟨⟨hcap, ⟨c, hc, ?_, hdc⟩, ⟨t, ht, ?_⟩, hv, hneeds, hfeat⟩, hj, ?_, hl⟩
  rotate_right
  · intro h
    apply hk
    simp only [Work.needCustomerKeys, List.any_nil, Bool.or_false, Bool.or_eq_true] at h ⊢
    rcases h with h | h <;> simp [h]
  · simp only [Work.effResidency, contains_foldRes, fold_deps_residency, Bool.and_eq_true,
      List.all_eq_true] at hres ⊢
    simp_all
  · have hs : (u.sensitivity.getD 0) ≤ u.effSensitivity := fold_deps_sensitivity _ _
    have hm := minTrust_mono hs
    simp only [Work.needTrust, Work.effSensitivity, List.foldl] at htrust ⊢
    simp only [Work.effSensitivity] at hm
    have h1 := Nat.max_le.mp htrust
    exact Nat.max_le.mpr ⟨Nat.le_trans hm h1.1, h1.2⟩

/-! ## Jurisdiction and key custody -/

/-- **No denied jurisdiction can compel plaintext.** If an offer is feasible
under a set of policies, and one of them denies jurisdiction `j`, and `j`'s
law reaches the offer (its own country, or a jurisdiction it declares),
then the customer holds the keys: compelled, the provider can hand over
only ciphertext. -/
theorem no_compelled_plaintext (u : Work Q) (ps : List Policy) (o : Offer Q)
    (hf : feasible u (effective ps) o) (p : Policy) (hp : p ∈ ps) (j : String)
    (hj : j ∈ p.denyJurisdictions) (hreach : o.country = some j ∨ j ∈ o.reach) :
    o.customerKeys = true := by
  have he := ((feasible_effective u ps o).mp hf).2 p hp |>.2.1 j hj
  unfold Offer.exposedTo at he
  cases hck : o.customerKeys
  · rcases hreach with hr | hr
    · simp [hck, hr] at he
    · simp [hck] at he
      exact absurd hr he.2
  · rfl

/-- **Required custody is enforced.** If a unit needs customer-held keys —
because it asks, because data it depends on does, or because a policy does —
every offer it may run on has them. -/
theorem customer_keys_enforced (u : Work Q) (eff : Effective) (o : Offer Q)
    (hf : feasible u eff o) (hk : u.needCustomerKeys eff = true) : o.customerKeys = true :=
  ((feasible_iff u eff o).mp hf).2.2.1 hk

/-- **Holding your own keys only helps.** An offer that is feasible with the
provider holding the keys stays feasible if the customer holds them
instead. -/
theorem custody_only_helps (u : Work Q) (eff : Effective) (o : Offer Q) :
    feasible u eff o → feasible u eff { o with customerKeys := true } := by
  rw [feasible_iff, feasible_iff]
  rintro ⟨hb, _, _, hl⟩
  exact ⟨hb, fun y _ => by simp [Offer.exposedTo], fun _ => rfl, hl⟩

/-! ## Latency -/

/-- **The latency target holds where the users are.** An offer a unit may
run on has, from every region its users are in, a measured p95 within the
target. -/
theorem latency_met (u : Work Q) (eff : Effective) (o : Offer Q) (hf : feasible u eff o)
    (t : Q) (hs : u.slo = some t) (r : String) (hr : r ∈ u.sloFrom) :
    ∃ h, lookupRes o.latency r = some h ∧ h ≤ t :=
  ((feasible_iff u eff o).mp hf).2.2.2 t hs r hr

/-- **Unmeasured fails closed.** An offer with no latency measurement from a
region the users are in is never allowed. -/
theorem unmeasured_excluded (u : Work Q) (eff : Effective) (o : Offer Q) (t : Q)
    (hs : u.slo = some t) (r : String) (hr : r ∈ u.sloFrom) (hl : lookupRes o.latency r = none) :
    ¬ feasible u eff o := by
  intro hf
  obtain ⟨h, hh, _⟩ := latency_met u eff o hf t hs r hr
  rw [hl] at hh
  exact Option.noConfusion hh

/-- **A target without users rules nothing out.** With no `from` regions,
the latency target excludes no offer: whatever is feasible is decided by
the other nine rules alone. -/
theorem no_users_no_latency_rule (u : Work Q) (eff : Effective) (o : Offer Q) (h : u.sloFrom = []) :
    latencyPass u o := by
  intro t _ r hr
  rw [h] at hr
  exact absurd hr (List.not_mem_nil r)

/-! ## Fail closed -/

/-- **A unit that deploys has somewhere legal to run.** -/
theorem deploys_has_feasible (u : Work Q) (eff : Effective) (os : List (Offer Q)) :
    deploys u eff os = true → ∃ o ∈ os, feasible u eff o := by
  unfold deploys
  intro h
  simp only [Bool.and_eq_true, List.any_eq_true, decide_eq_true_eq] at h
  exact h.1

/-- **A unit that deploys meets its redundancy.** If it asks for `n` vendors,
at least `n` distinct vendors have a feasible offer. -/
theorem deploys_meets_redundancy (u : Work Q) (eff : Effective) (os : List (Offer Q)) (n : Nat)
    (hn : u.minVendors = some n) :
    deploys u eff os = true → n ≤ (feasibleVendors u eff os).length := by
  unfold deploys
  intro h
  simp only [hn, Bool.and_eq_true, decide_eq_true_eq] at h
  exact h.2

/-- **No feasible offer, no deployment.** -/
theorem no_feasible_no_deploy (u : Work Q) (eff : Effective) (os : List (Offer Q))
    (h : ∀ o ∈ os, ¬ feasible u eff o) : deploys u eff os = false := by
  cases hd : deploys u eff os
  · rfl
  · obtain ⟨o, ho, hf⟩ := deploys_has_feasible u eff os hd
    exact absurd hf (h o ho)

end Bailiwick

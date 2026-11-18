/-!
# The Bailiwick model

The rules that decide which offers a unit of a workload (a service or a job)
may run on, written to mirror the reference implementation
(`src/semantics.rs`, `src/policy.rs`, `src/feasibility.rs`) step for step.

Locations are modelled structurally rather than as strings: a location is a
country (`de`) or a subdivision of one (`us-ca`). Region *names* are not
modelled; a residency is the set of locations a name stands for. The
executable (`Main.lean`) and the differential test check that the string
level agrees.

Resource amounts are any type with a decidable `≤` (the executable uses
`Float`, as the implementation does); no theorem depends on arithmetic.
-/

namespace Bailiwick

/-! ## Locations and residency -/

/-- A location: a whole country, or a subdivision of one. -/
inductive Loc where
  | country (c : String)
  | sub (c s : String)
  deriving DecidableEq, Repr

/-- A residency constraint: unconstrained, or an explicit set of locations.
A country in the set covers all of its subdivisions. -/
inductive Residency where
  | global
  | set (locs : List Loc)
  deriving Repr

/-- Whether the set `s` covers location `l`: directly, or through its
country (`semantics.rs`, `covers`). -/
def covers (s : List Loc) : Loc → Bool
  | .country c => decide (Loc.country c ∈ s)
  | .sub c d => decide (Loc.sub c d ∈ s) || decide (Loc.country c ∈ s)

/-- Drop a subdivision whose country is also present: the country already
covers it (`semantics.rs`, `normalize`). -/
def normalize (s : List Loc) : List Loc :=
  s.filter fun
    | .country _ => true
    | .sub c _ => !decide (Loc.country c ∈ s)

/-- Intersection of two residencies (`Residency::intersect`). -/
def Residency.inter : Residency → Residency → Residency
  | .global, x => x
  | .set a, .global => .set a
  | .set a, .set b => .set (normalize (a.filter (covers b) ++ b.filter (covers a)))

/-- Whether an offer in `country`, with subdivision `sub` if known, lies inside
the residency. A sub-national residency never admits an offer whose
subdivision is unknown (`Residency::contains`). -/
def Residency.contains : Residency → String → Option String → Bool
  | .global, _, _ => true
  | .set s, c, sub =>
    decide (Loc.country c ∈ s) ||
      match sub with
      | some d => decide (Loc.sub c d ∈ s)
      | none => false

/-- Fold an optional constraint into an accumulated one; `none` means
"unconstrained so far" (`fold_residency`, `fold_residencies`). -/
def foldRes : Option Residency → Option Residency → Option Residency
  | none, b => b
  | a, none => a
  | some a, some b => some (a.inter b)

/-- `none` behaves as `global`. -/
def Residency.ofOption : Option Residency → Residency
  | none => .global
  | some r => r

/-! ## Sensitivity and trust -/

/-- The minimum trust level for a sensitivity: S0 → T1, S1 → T2, S2 → T2,
S3 → T3 (`min_trust`). -/
def minTrust : Nat → Nat
  | 0 => 1
  | 1 => 2
  | 2 => 2
  | _ => 3

/-! ## Policies -/

/-- An organization policy (`ast::Policy`). Budgets are not modelled: they
cap spending and never decide feasibility. -/
structure Policy where
  name : String
  residency : Option Residency
  minTrust : Option Nat
  denyVendors : List String
  denyCountries : List String
  denyJurisdictions : List String
  customerKeys : Bool

/-- The policies in force, composed (`policy::effective`): residency by
intersection, trust floor by maximum, denials by union. A denial remembers
the first policy that made it. -/
structure Effective where
  residency : Option Residency
  minTrust : Nat
  denyVendors : List (String × String)
  denyCountries : List (String × String)
  denyJurisdictions : List (String × String)
  customerKeys : Bool

def Effective.empty : Effective := ⟨none, 0, [], [], [], false⟩

/-- Add a denial unless the item is already denied (first policy wins). -/
def addDeny (acc : List (String × String)) (policy : String) (items : List String) :
    List (String × String) :=
  items.foldl (fun acc x => if acc.any (·.1 == x) then acc else acc ++ [(x, policy)]) acc

def Effective.add (e : Effective) (p : Policy) : Effective where
  residency := foldRes e.residency p.residency
  minTrust := max e.minTrust (p.minTrust.getD 0)
  denyVendors := addDeny e.denyVendors p.name p.denyVendors
  denyCountries := addDeny e.denyCountries p.name p.denyCountries
  denyJurisdictions := addDeny e.denyJurisdictions p.name p.denyJurisdictions
  customerKeys := e.customerKeys || p.customerKeys

def effective (ps : List Policy) : Effective := ps.foldl Effective.add Effective.empty

/-- The first policy denying `x`, if any. -/
def deniedBy (ds : List (String × String)) (x : String) : Option String :=
  (ds.find? (·.1 == x)).map (·.2)

/-! ## Offers, units and the seven rules -/

variable {Q : Type} [LE Q] [DecidableRel (α := Q) (· ≤ ·)]

/-- Somewhere a unit could run (`ast::Offer`). -/
structure Offer (Q : Type) where
  id : String
  vendor : Option String
  country : Option String
  sub : Option String
  trust : Option Nat
  classes : List String
  resources : List (String × Q)
  features : List String
  /-- Jurisdictions, besides the offer's own country, whose law reaches its
  operator. -/
  reach : List String
  /-- Whether the customer, not the provider, holds the keys. -/
  customerKeys : Bool
  /-- Measured p95 latency from users in each region. -/
  latency : List (String × Q)

/-- A data block a service depends on. -/
structure Dep where
  residency : Option Residency
  sensitivity : Option Nat
  customerKeys : Bool

/-- A unit of a workload: a service or a job. A job has no dependencies and
requires no features. -/
structure Work (Q : Type) where
  cls : String
  residency : Option Residency
  sensitivity : Option Nat
  deps : List Dep
  needs : List (String × Q)
  features : List String
  minVendors : Option Nat
  customerKeys : Bool
  /-- The p95 latency target, and the regions it must be met from. -/
  slo : Option Q
  sloFrom : List String

/-- Why an offer was excluded. -/
inductive Exclusion (Q : Type) where
  | capability (cls : String)
  | residency (country : String)
  | trust (amount need : Nat)
  | policyVendor (vendor policy : String)
  | policyCountry (country policy : String)
  | resource (name : String) (need : Q) (amount : Option Q)
  | feature (name : String)
  | jurisdiction (j policy : String)
  | custody
  | latency (region : String)
  | missingField (field : String)

/-- The unit's residency once its dependencies and the policies are folded
in (`plan_service`, `plan_job`). -/
def Work.effResidency (u : Work Q) (eff : Effective) : Residency :=
  .ofOption (foldRes (u.deps.foldl (fun acc d => foldRes acc d.residency) u.residency) eff.residency)

/-- The unit's sensitivity once its dependencies are folded in. -/
def Work.effSensitivity (u : Work Q) : Nat :=
  u.deps.foldl (fun s d => max s (d.sensitivity.getD 0)) (u.sensitivity.getD 0)

/-- The trust the unit needs. -/
def Work.needTrust (u : Work Q) (eff : Effective) : Nat :=
  max (minTrust u.effSensitivity) eff.minTrust

/-- Whether the unit needs customer-held keys: it asks for them, a data
dependency does, or a policy does. -/
def Work.needCustomerKeys (u : Work Q) (eff : Effective) : Bool :=
  u.customerKeys || u.deps.any (·.customerKeys) || eff.customerKeys

/-- Whether jurisdiction `j` could compel plaintext from the offer: its law
reaches the operator (the offer's own country, or a declared
jurisdiction), and the provider holds the keys. -/
def Offer.exposedTo (o : Offer Q) (j : String) : Bool :=
  !o.customerKeys && (o.country == some j || o.reach.contains j)

/-- The amount of resource `n` an offer declares, if it does. -/
def lookupRes (rs : List (String × Q)) (n : String) : Option Q :=
  (rs.find? (·.1 == n)).map (·.2)

/-- Rule 10: from every region the users are in, a measured p95 within the
target. An offer with no measurement from a region is excluded: unmeasured
fails closed. -/
def latencyExcl (u : Work Q) (o : Offer Q) : List (Exclusion Q) :=
  match u.slo with
  | none => []
  | some t => u.sloFrom.filterMap fun r =>
    match lookupRes o.latency r with
    | some h => if h ≤ t then none else some (Exclusion.latency r)
    | none => some (Exclusion.latency r)

/-- The ten rules, in the implementation's order (`eval_offer`):
1. capability, 2. residency, 3. policy country denial, 4. trust,
5. policy vendor denial, 6. resources, 7. features, 8. jurisdiction,
9. key custody, 10. latency. -/
def evalOffer (u : Work Q) (eff : Effective) (o : Offer Q) : List (Exclusion Q) :=
  let res := u.effResidency eff
  let need := u.needTrust eff
  let capability := if o.classes.contains u.cls then [] else [Exclusion.capability u.cls]
  let location := match o.country with
    | none => [Exclusion.missingField "country"]
    | some c =>
      (if res.contains c o.sub then [] else [Exclusion.residency c]) ++
      (match deniedBy eff.denyCountries c with
       | some p => [Exclusion.policyCountry c p]
       | none => [])
  let trust := match o.trust with
    | none => [Exclusion.missingField "trust"]
    | some t => if t < need then [Exclusion.trust t need] else []
  let vendor := match o.vendor with
    | none => []
    | some v => match deniedBy eff.denyVendors v with
      | some p => [Exclusion.policyVendor v p]
      | none => []
  let resources := u.needs.filterMap fun (n, need) =>
    match lookupRes o.resources n with
    | some amount => if need ≤ amount then none else some (Exclusion.resource n need (some amount))
    | none => some (Exclusion.resource n need none)
  let features := u.features.filterMap fun f =>
    if o.features.contains f then none else some (Exclusion.feature f)
  let jurisdiction := eff.denyJurisdictions.filterMap fun (j, p) =>
    if o.exposedTo j then some (Exclusion.jurisdiction j p) else none
  let custody := if u.needCustomerKeys eff && !o.customerKeys then [Exclusion.custody] else []
  capability ++ location ++ trust ++ vendor ++ resources ++ features ++ jurisdiction ++ custody ++
    latencyExcl u o

/-- An offer is feasible when no rule excludes it. -/
def feasible (u : Work Q) (eff : Effective) (o : Offer Q) : Prop :=
  (evalOffer u eff o).isEmpty = true

instance (u : Work Q) (eff : Effective) (o : Offer Q) : Decidable (feasible u eff o) :=
  inferInstanceAs (Decidable ((evalOffer u eff o).isEmpty = true))

/-- The distinct vendors among the feasible offers. -/
def feasibleVendors (u : Work Q) (eff : Effective) (os : List (Offer Q)) : List String :=
  ((os.filter fun o => decide (feasible u eff o)).filterMap (·.vendor)).eraseDups

/-- A unit deploys unless it fails closed: it needs at least one feasible
offer, and as many distinct feasible vendors as its redundancy asks for
(`UnitPlan::fails_closed`). -/
def deploys (u : Work Q) (eff : Effective) (os : List (Offer Q)) : Bool :=
  os.any (fun o => decide (feasible u eff o)) &&
    match u.minVendors with
    | none => true
    | some n => decide (n ≤ (feasibleVendors u eff os).length)

end Bailiwick

import Lean.Data.Json
import Bailiwick

/-!
# `bwk-model`: the proven model, runnable

Reads one case per line on standard input (JSON), evaluates it with the
same definitions the theorems are about, and writes one verdict per line.
`bwk-model verify` instead checks a deployment certificate (see below).
The Rust test `tests/lean_model.rs` sends it thousands of generated cases
and requires the implementation to agree on every one.

A case: `{"policies": [...], "unit": {...}, "offers": [...]}`; locations are
strings (`de`, `us-ca`), a residency is a list of them or `null`.
A verdict: `{"verdicts": [[exclusion, ...], ...], "deploys": bool}`.
-/

open Lean Bailiwick

def parseLoc (s : String) : Loc :=
  match s.splitOn "-" with
  | [c, d] => .sub c d
  | _ => .country s

def getField (j : Json) (k : String) : Json := j.getObjValD k

def optStr (j : Json) : Option String := (j.getStr?).toOption

def flag (j : Json) : Bool := ((j.getBool?).toOption).getD false

def optNat (j : Json) : Option Nat := (j.getNat?).toOption

def strs (j : Json) : List String :=
  match j.getArr? with
  | .ok a => a.toList.filterMap optStr
  | .error _ => []

def optRes (j : Json) : Option Residency :=
  match j.getArr? with
  | .ok a => some (.set (a.toList.filterMap optStr |>.map parseLoc))
  | .error _ => none

def toFloat (j : Json) : Float :=
  match j.getNum? with
  | .ok n => n.toFloat
  | .error _ => 0

def amounts (j : Json) : List (String × Float) :=
  match j.getArr? with
  | .ok a => a.toList.filterMap fun p =>
      match p.getArr? with
      | .ok #[n, v] => (optStr n).map (·, toFloat v)
      | _ => none
  | .error _ => []

def parsePolicy (j : Json) : Policy where
  name := (optStr (getField j "name")).getD ""
  residency := optRes (getField j "residency")
  minTrust := optNat (getField j "min_trust")
  denyVendors := strs (getField j "deny_vendors")
  denyCountries := strs (getField j "deny_countries")
  denyJurisdictions := strs (getField j "deny_jurisdictions")
  customerKeys := flag (getField j "customer_keys")

def parseDep (j : Json) : Dep where
  residency := optRes (getField j "residency")
  sensitivity := optNat (getField j "sensitivity")
  customerKeys := flag (getField j "customer_keys")

def parseWork (j : Json) : Work Float where
  cls := (optStr (getField j "class")).getD ""
  residency := optRes (getField j "residency")
  sensitivity := optNat (getField j "sensitivity")
  deps := match (getField j "deps").getArr? with
    | .ok a => a.toList.map parseDep
    | .error _ => []
  needs := amounts (getField j "needs")
  features := strs (getField j "features")
  minVendors := optNat (getField j "min_vendors")
  customerKeys := flag (getField j "customer_keys")
  slo := match (getField j "slo").getNum? with
    | .ok n => some n.toFloat
    | .error _ => none
  sloFrom := strs (getField j "slo_from")

def parseOffer (j : Json) : Offer Float where
  id := (optStr (getField j "id")).getD ""
  vendor := optStr (getField j "vendor")
  country := optStr (getField j "country")
  sub := optStr (getField j "sub")
  trust := optNat (getField j "trust")
  classes := strs (getField j "classes")
  resources := amounts (getField j "resources")
  features := strs (getField j "features")
  reach := strs (getField j "jurisdictions")
  customerKeys := flag (getField j "customer_keys")
  latency := amounts (getField j "latency")

/-- An exclusion as the implementation names it: its kind, and the detail
that identifies it. -/
def render : Exclusion Float → String
  | .capability _ => "capability"
  | .residency _ => "residency"
  | .trust a n => s!"trust:{a}<{n}"
  | .policyVendor _ p => s!"policy_vendor:{p}"
  | .policyCountry _ p => s!"policy_country:{p}"
  | .resource n _ _ => s!"resource:{n}"
  | .feature f => s!"feature:{f}"
  | .jurisdiction j p => s!"jurisdiction:{j}:{p}"
  | .custody => "key_custody"
  | .latency r => s!"latency:{r}"
  | .missingField f => s!"missing_field:{f}"

def evalCase (j : Json) : Json :=
  let policies := match (getField j "policies").getArr? with
    | .ok a => a.toList.map parsePolicy
    | .error _ => []
  let u := parseWork (getField j "unit")
  let offers := match (getField j "offers").getArr? with
    | .ok a => a.toList.map parseOffer
    | .error _ => []
  let eff := effective policies
  let verdicts := offers.map fun o =>
    Json.arr ((evalOffer u eff o).map (Json.str ∘ render)).toArray
  Json.mkObj [("verdicts", Json.arr verdicts.toArray), ("deploys", Json.bool (deploys u eff offers))]

partial def loop (stdin : IO.FS.Stream) (stdout : IO.FS.Stream) : IO Unit := do
  let line ← stdin.getLine
  if line.isEmpty then return
  let line := line.trimRight
  if line.isEmpty then loop stdin stdout else
  match Json.parse line with
  | .ok j => stdout.putStrLn (evalCase j).compress
  | .error e => stdout.putStrLn (Json.mkObj [("error", Json.str e)]).compress
  stdout.flush
  loop stdin stdout

/-! ## `bwk-model verify`: check a deployment certificate

A certificate (written by `bailiwick certify`) carries, for every unit, the
case its decision depended on and the claimed outcome. Each claim is
recomputed here with the definitions the theorems are about, so a
certificate this accepts inherits them: every placement is `feasible`, and
therefore (`feasible_effective`, `no_compelled_plaintext`, ...) satisfies
every policy and is beyond the reach of every denied jurisdiction. -/

def strList (j : Json) : List String := strs j

/-- Check one unit; `none` if every claim holds, else what is wrong. -/
def checkUnit (u : Json) : Option String :=
  let name := s!"{(optStr (getField u "workload")).getD "?"}.{(optStr (getField u "name")).getD "?"}"
  let c := getField u "case"
  let got := evalCase c
  let offers := match (getField c "offers").getArr? with
    | .ok a => a.toList.map parseOffer
    | .error _ => []
  let policies := match (getField c "policies").getArr? with
    | .ok a => a.toList.map parsePolicy
    | .error _ => []
  let w := parseWork (getField c "unit")
  let eff := effective policies
  let feasibleIds := (offers.filter fun o => decide (feasible w eff o)).map (·.id)
  if (getField got "verdicts").compress != (getField u "verdicts").compress then
    some s!"{name}: the verdicts are not what the model computes"
  else if (getField got "deploys").compress != (getField u "deploys").compress then
    some s!"{name}: `deploys` is not what the model computes"
  else if strList (getField u "feasible") != feasibleIds then
    some s!"{name}: the feasible set is not what the model computes"
  else
    match (getField u "placement").getArr? with
    | .error _ => none
    | .ok ps =>
      let bad := ps.toList.filterMap optStr |>.filter (· ∉ feasibleIds)
      if bad.isEmpty then none
      else some s!"{name}: placed on {bad}, which the model does not find feasible"

partial def readAll (stdin : IO.FS.Stream) (acc : String) : IO String := do
  let line ← stdin.getLine
  if line.isEmpty then return acc else readAll stdin (acc ++ line)

def isTrue (j : Json) : Bool := ((j.getBool?).toOption) == some true

def verify (stdin stdout : IO.FS.Stream) : IO UInt32 := do
  let text ← readAll stdin ""
  match Json.parse text with
  | .error e =>
    stdout.putStrLn s!"not JSON: {e}"
    return 1
  | .ok cert =>
    if (optStr (getField cert "format")) != some "bailiwick-certificate/1" then
      stdout.putStrLn "not a bailiwick-certificate/1"
      return 1
    let units := match (getField cert "units").getArr? with
      | .ok a => a.toList
      | .error _ => []
    let problems := units.filterMap checkUnit
    let deployable := units.all fun u => isTrue (getField u "deploys")
    let deployableOk := ((getField cert "deployable").getBool?).toOption == some deployable
    let placed := (units.filter fun u => ((getField u "placement").getArr?).toOption.isSome).length
    for p in problems do stdout.putStrLn s!"invalid: {p}"
    if !deployableOk then stdout.putStrLn "invalid: `deployable` does not follow from the units"
    if problems.isEmpty && deployableOk then
      stdout.putStrLn s!"valid: {units.length} unit(s), {placed} placement(s); every verdict reproduced by the proven model"
      return 0
    else
      return 1

def main (args : List String) : IO UInt32 := do
  match args with
  | ["verify"] => verify (← IO.getStdin) (← IO.getStdout)
  | [] =>
    loop (← IO.getStdin) (← IO.getStdout)
    return 0
  | _ =>
    IO.eprintln "usage: bwk-model            (cases on stdin, one per line)\n       bwk-model verify     (a certificate on stdin)"
    return 2

import Lake
open Lake DSL

-- Bailiwick's semantics in Lean 4: a model of the rules that decide
-- which offers a workload may run on, the theorems that hold of it, and an
-- executable of the same model that the Rust implementation is checked
-- against. No dependencies beyond Lean itself.

package bailiwick where

@[default_target]
lean_lib Bailiwick where

lean_exe «bwk-model» where
  root := `Main

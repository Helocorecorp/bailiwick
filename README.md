<p align="center">
  <img src="assets/mark.svg" alt="Bailiwick" width="160">
</p>

<h1 align="center">Bailiwick</h1>

<p align="center"><strong>Say what your software needs. Not where it runs.</strong></p>

---

Bailiwick is a small, typed language for describing what a workload *requires*: where its data may live, how sensitive it is, how fast it must answer, how many providers it must survive losing, and how much machine it needs. It never names a cloud, a region or a server. Those are choices; Bailiwick is the contract they have to honour.

```bwk
workload shop {
  data customers {
    sensitivity = S3            // regulated personal data
    residency   = eu
  }

  service api : backend-api {
    redundancy { vendors >= 2 }
    resources  { vcpu >= 2  mem >= 4 GB }
    depends_on  = customers     // inherits EU residency and S3
  }
}
```

Against a catalog of offers (here, fictional ones from `examples/catalog/`):

```text
$ bailiwick explain shop.bwk --catalog offers.bwk
workload shop
  service api [backend-api]  residency=eu sensitivity=S3 min_trust=T3
    requires: vcpu>=2 mem>=4GB
    feasible: b-fr, c-fr  (2 vendor(s))
    excluded a-de: trust T2 is below the required T3
    excluded e-us-va: country `us` is outside residency `eu`
    requires >= 2 vendor(s)
  => deployable
```

## What it gives you

- **Typed units.** Durations, data sizes, money and counts are different types: `slo.p95 <= 5 GB` is an error, not a typo that ships.
- **Policies that can only narrow.** An organization's `policy` blocks compose by intersection: importing one can never loosen anything.
- **Requirements that flow.** A service that depends on regulated data inherits that data's residency and sensitivity.
- **Latency where your users are, measured.** `slo.p95 <= 200 ms from eu` is a rule, not a wish: an offer qualifies only with a measured p95 from the EU within 200 ms, and an offer with no measurement is excluded.
- **Whose law reaches your data, and who holds the keys.** Offers declare the jurisdictions that reach their operator and who holds the encryption keys. A policy can deny a jurisdiction: an offer within its reach stays allowed only if *you* hold the keys, because then a court order yields ciphertext. Data can require customer-held keys outright.
- **Answers with reasons.** Given offers (places software could run), the toolchain says which are allowed and, for every one that isn't, exactly why.
- **A way in from what you have.** Importers turn `vendor-u.toml`, Docker Compose, Kubernetes manifests and Terraform into Bailiwick, marking every inferred requirement `TODO: confirm` with the line it came from.
- **Answers for every provider that could ever exist.** `bailiwick prove` shows a program already satisfies GDPR, or that a pull request loosens no rule, for every possible offer, not just today's catalog. When the answer is no, it shows the exact offer that breaks it.
- **Certificates an auditor checks offline.** `bailiwick certify` records every decision with the exact inputs behind it. The program built from the Lean proofs re-checks it, so an auditor needs neither your systems nor your word.
- **Deterministic and auditable.** No dependencies, no network access. The same input gives byte-identical output, as JSON or Protobuf, and a conformance suite pins every verdict.

## Proven, not just tested

Bailiwick's semantics are modelled in [Lean 4](https://lean-lang.org) ([`formal/`](formal/)), and its core guarantees are theorems checked by Lean's kernel:

| Guarantee | Theorem |
|---|---|
| An offer is excluded **exactly** when one of the ten rules fails, never for any other reason | `feasible_iff` |
| **Policies only narrow**: adding a policy never makes a forbidden offer allowed | `policies_only_narrow` |
| **Order doesn't matter**: the same policies in any order, with any repetition, give the same verdicts | `policy_order_irrelevant` |
| Under any set of policies, an offer is feasible **iff** it is feasible with none and each policy alone admits it | `feasible_effective` |
| **Region intersection is exact**, including countries covering their subdivisions | `contains_inter` |
| **Dependencies only add requirements**: depending on regulated data never loosens anything | `dependencies_only_raise` |
| **No denied jurisdiction can compel plaintext**: if a policy denies a jurisdiction whose law reaches an allowed offer, the customer holds that offer's keys | `no_compelled_plaintext` |
| **Required custody is enforced**: if data, a service or a policy needs customer-held keys, every allowed offer has them | `customer_keys_enforced` |
| **Holding your own keys only helps**: it never makes an allowed offer forbidden | `custody_only_helps` |
| **Latency holds where the users are**: an allowed offer has a measured p95 within the target from every region the users are in | `latency_met` |
| **Unmeasured fails closed**: an offer with no measurement from a user region is never allowed | `unmeasured_excluded` |
| **Adopting a policy changes nothing exactly when** everything already allowed satisfies it, which is what `prove --implies` decides | `adopting_changes_nothing_iff` |
| **Fail closed**: a unit that deploys has a legal place to run and meets its redundancy | `deploys_has_feasible`, `deploys_meets_redundancy` |

The proofs rest only on Lean's standard axioms, with no unproven steps.

And the proofs are about the code you run, not just a model of it: the model is also an executable, and on every change CI generates 20,000 random Bailiwick programs, runs each through this implementation and through the proven model, and requires identical verdicts for every offer, reason by reason.

## Add it to your project

```sh
bailiwick init            # writes bailiwick.bwk from your vendor-u.toml, Compose, Kubernetes or Terraform files
bailiwick explain bailiwick.bwk --catalog offers.bwk --policy gdpr
```

`init` finds the deployment files a project already has and writes one `bailiwick.bwk`, marking every inferred requirement `TODO: confirm`. With none, it writes a starting point.

**Rule packs.** Laws are built in. Write `import "gdpr"` (or pass `--policy gdpr`) and the policy applies to everything in the file:

| Pack | What it enforces |
|---|---|
| `gdpr` | EU: the EEA or a country with an EU adequacy decision |
| `eu-sovereign` | The EU, and nothing within reach of US, Chinese or Russian law unless you hold the keys |
| `uk-gdpr` | The UK or a country with UK adequacy |
| `pipeda` | Canada: audited, contracted providers (PIPEDA does not restrict location) |
| `canada-residency` | Canada only |
| `quebec-law25` | Québec only |
| `hipaa` | US health data: audited, contracted providers (HIPAA does not restrict location) |
| `ccpa` | California: audited, contracted providers |
| `lgpd` | Brazil |
| `pipl` | China |
| `rbi-payments` | India, for payment system data |
| `my-health-records` | Australia, for My Health Record data |

Each pack's header says what the law requires, what the pack enforces and what it cannot check (contracts such as standard clauses or a business associate agreement). The packs are a starting point, not legal advice.

**Check real infrastructure.** `check-plan` holds a Terraform plan or Kubernetes manifests to the same rules, resource by resource:

```sh
terraform show -json plan.out > plan.json
bailiwick check-plan bailiwick.bwk plan.json
bailiwick check-plan bailiwick.bwk k8s/*.yaml --vendor vendor-e
```

A resource belongs to a service, job or data block by its `bailiwick` tag or label, or by having its name.

**On every pull request.** The GitHub Action comments where each part may run and whether the plan follows the rules, and fails the check when it does not:

```yaml
permissions:
  pull-requests: write
steps:
  - uses: actions/checkout@v4
  - uses: Helocorecorp/bailiwick@main
    with:
      policy: gdpr
      plan: plan.json            # optional: a Terraform plan or Kubernetes manifests
```

**Without Rust.** The same toolchain, compiled to WebAssembly:

```sh
npx @helocore/bailiwick explain bailiwick.bwk --catalog offers.bwk --policy gdpr
pip install bailiwick-lang && bailiwick explain bailiwick.bwk --catalog offers.bwk --policy gdpr
```

## Prove it, for every provider

`explain` answers for the offers in a catalog. `prove` answers for every offer any provider could ever make:

```text
$ bailiwick prove shop.bwk --implies eu-sovereign
workload shop  (for every offer any provider could make)
  REFUTED  service `web`: everything it allows satisfies `eu-sovereign`
           counterexample: an offer in at, trust T3, reachable by us law, provider-held keys
             within reach of jurisdiction `us`, denied by policy `eu-sovereign`, and the provider holds the keys
  proven   service `media`: everything it allows satisfies `eu-sovereign`
```

- `bailiwick prove app.bwk`: can each service and job run anywhere at all? If its requirements contradict each other, no provider will ever be able to host it, and it says which rules collide.
- `bailiwick prove app.bwk --implies gdpr`: does everything the program allows already satisfy a policy?
- `bailiwick prove app.bwk --no-looser-than old.bwk`: does a change allow nothing the old version refused? The GitHub Action runs this on every pull request against the base branch and shows the offer a loosening would let in.

It can speak for every possible offer because the rules only ever compare an offer to values the program mentions. Two offers that agree on every such comparison get the same verdict, so trying one offer of each kind covers them all. Every counterexample it reports is a real offer, re-checked by the evaluator. A test throws 600,000 random concrete offers at it, and it has never been contradicted.

## Deployment certificates

```sh
bailiwick certify app.bwk --catalog offers.bwk --place api=e-de-he-ck > cert.json
bwk-model verify < cert.json                                  # re-checked by the proven model
bailiwick verify cert.json app.bwk --catalog offers.bwk       # and against the source
```

A certificate holds, for each service and job, the inputs its decision depended on, every offer's verdict and where it was placed, bound by a SHA-256 digest of the inputs. `certify` refuses a placement that breaks any rule. `bwk-model` (built from [`formal/`](formal/)) recomputes every verdict with the definitions the theorems are about, so a certificate it accepts carries those theorems with it: each placement satisfies every policy, and no denied jurisdiction can compel plaintext from it.

## Try it in your browser

The [playground](playground/) is this toolchain compiled to WebAssembly: write a program and see, as you type, where each part may run and why, with its certificate. Nothing is sent anywhere. Build it with `playground/build.sh` and serve `playground/site/`.

## Install

**Editor.** Install *Bailiwick* from the VS Code Marketplace (or Open VSX for Cursor, VSCodium and Windsurf). The language server is bundled: errors as you type, completion, hover, go-to-definition, outline and formatting.

**Command line.** With a Rust toolchain:

```sh
cargo install --git https://github.com/Helocorecorp/bailiwick
```

## Use

```sh
bailiwick init                                   # start: write bailiwick.bwk for this project
bailiwick check   app.bwk [--policy org.bwk]     # check a file
bailiwick fmt     app.bwk --write                # format in place
bailiwick explain app.bwk --catalog offers.bwk   # what is allowed, and why not
bailiwick check-plan app.bwk plan.json           # hold a Terraform plan or k8s manifests to the rules
bailiwick certify app.bwk --catalog offers.bwk   # a certificate of every decision
bailiwick verify  cert.json app.bwk --catalog offers.bwk
bailiwick import  terraform main.tf              # also: vendor-u, compose, k8s
bailiwick ir      app.bwk [--proto]              # canonical form, JSON or Protobuf
bailiwick lsp                                    # language server over stdio
```

Exit codes: `0` success, `1` diagnostics or a plan that cannot deploy, `2` usage.

## The language

A tour of workloads, services, jobs, data, residency regions, sensitivity and trust, redundancy, policies and offers is in the [extension's guide](editors/vscode/README.md). The precise grammar is [`spec/grammar.ebnf`](spec/grammar.ebnf), and the canonical forms are [`spec/ir/`](spec/ir/).

## Repository

| | |
|---|---|
| `src/` | Lexer, parser with unit checking, semantics, policy composition, feasibility, IR (JSON and Protobuf), importers, formatter, language server |
| `spec/` | Grammar, IR schemas, and the conformance suite every implementation must reproduce |
| `examples/catalog/` | Fictional example offers (made-up vendors) for trying the language |
| `examples/` | Example workloads, policies and import sources |
| `editors/vscode/` | The VS Code extension |
| `editors/tree-sitter-bailiwick/` | A tree-sitter grammar |
| `formal/` | The semantics in Lean 4, the proofs, and `bwk-model` |
| `playground/` | The toolchain in WebAssembly, and the browser playground |
| `packs/` | The built-in compliance rule packs |
| `packages/` | The npm and Python packages |
| `action.yml` | The GitHub Action |

```sh
cargo test                     # the whole suite, including a live language-server session
```

## Contributing

Pull requests are welcome. Before one can be merged, its author signs the [Helocore Contributor License Agreement](CLA.md) once, by replying to the pull request when the CLA bot asks. Every change runs the full test suite and the conformance check.

## License

Copyright 2026 Helocore Corp. Written by Phantombyter.

Licensed under the [Apache License 2.0](LICENSE).

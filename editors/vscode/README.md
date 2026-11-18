# Bailiwick

**Say what your software needs. Not where it runs.**

Bailiwick is a small, typed language for describing what a workload *requires*: where its data may live, how sensitive it is, how fast it must answer, how many providers it must survive losing, how much machine it needs. It never says *which* cloud, region or server. Those are decisions for later. Bailiwick is the contract they have to honour.

This extension gives you the full language in VS Code: highlighting, errors as you type, completion, hover documentation, go-to-definition, an outline, and formatting.

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

`api` never mentions the EU or a sensitivity level. It depends on `customers`, so it inherits both. Change the data's rules and everything that touches it follows.

---

## Why

Infrastructure files mix two different things: *requirements* ("customer data stays in the EU") and *choices* ("run in `eu-west-1`"). Over time the choices are all that's left, and nobody can say which of them are actually required.

Bailiwick keeps the requirements on their own, where they can be read, reviewed, checked and enforced:

- **Typed units.** Durations, data sizes, money and counts are different types. `slo.p95 <= 5 GB` is an error, not a typo that ships.
- **Rules that compose safely.** An organization-wide `policy` can only *narrow* what a workload may do. Importing a policy can never loosen anything.
- **Requirements that flow.** A service that depends on regulated data inherits that data's residency and sensitivity automatically.
- **Explainable answers.** Given a list of places your software could run, the toolchain tells you which are allowed, and for every one that isn't, exactly why.

---

## Features

| | |
|---|---|
| **Diagnostics** | Syntax and type errors, unknown regions and classes, unit mistakes, unresolved references, workloads that contradict the policies in force, as you type. |
| **Completion** | Keywords, fields, regions, asset classes, profiles, sensitivity and trust levels, units. |
| **Hover** | What every keyword, region, class and level means, right where you use it. |
| **Go to definition** | From `depends_on = customers` to the `data customers` it names. |
| **Outline** | Workloads, services, jobs, data and policies in the Outline view and breadcrumbs. |
| **Formatting** | One canonical layout (*Format Document*). |
| **Highlighting** | Full syntax highlighting for `.bwk` files. |

The language server ships inside the extension for Windows, macOS (Intel and Apple Silicon) and Linux. There's nothing else to install.

---

## A quick tour

### Workloads, services and jobs

A **workload** is one application. It contains **services** (long-running, answering requests), **jobs** (batch work with a deadline) and **data** (what the application stores).

```bwk
workload reports {
  profile = balanced
  budget  = $2000 / month

  service web : frontend-static {
    cache "/assets/*" for 30 d
    cache "/api/*" never
  }

  job nightly : background-jobs {
    deadline       = 6 h
    checkpointable = true
    resources      { vcpu >= 4  mem >= 8 GB }
  }
}
```

Every service and job has a **class**: `frontend-static`, `backend-api`, `media-assets`, `database`, `background-jobs` or `ml-inference`.

### Residency

`residency` says where something may run. Regions are sets of countries, and Bailiwick knows how they relate: `eu` is inside `eea`; `eu` and `nordics` overlap in Denmark, Finland and Sweden; `california` is inside `us`.

`global` · `eu` · `eea` · `nordics` · `us` · `ca` · `uk` · `apac` · `california` · `quebec`

### Sensitivity and trust

Data is classified from `S0` (public) to `S3` (regulated). Each level demands a minimum **trust** level from wherever it runs: `T1` may hold `S0`, `T2` up to `S2`, and `T3` up to `S3`. Declare the sensitivity and the trust requirement follows.

### Redundancy, latency and resources

```bwk
service checkout : backend-api {
  slo.p95    <= 150 ms from eu         // 95% of requests from EU users answered within 150 ms
  redundancy { vendors >= 2 }          // survive losing an entire provider
  resources  { vcpu >= 2  mem >= 4 GB  disk >= 20 GB }
  requires   = [ websockets ]          // capabilities the platform must offer
}
```

### Policies

A **policy** is a rule set for a whole organization. Policies only narrow: residency intersects, trust floors rise, denied vendors and countries are removed, budgets are capped.

```bwk
// policies/eu-only.bwk
policy eu-only {
  residency  = eu
  min_trust  = T2
  deny countries = [ ru, by ]
  budget    <= $50000 / month
}
```

```bwk
import "policies/eu-only.bwk"

workload storefront {
  service web : backend-api { }       // EU only and at least T2, because of the policy
}
```

### Jurisdiction and key custody

Where a server sits is one question; whose law can compel its operator is another. An offer lists the jurisdictions that reach its operator besides its own country, and who holds the encryption keys (unstated means the provider):

```bwk
offer e-de-he-ck {
  vendor        = vendor-e
  country       = de
  jurisdictions = [ us ]       // US-headquartered operator
  keys          = customer     // keys in the customer's own key store
}
```

A policy can deny jurisdictions. An offer within reach of a denied jurisdiction is allowed only if the customer holds the keys, because a court order then yields only ciphertext. Data, a service, a job or a policy can also require customer-held keys outright:

```bwk
policy sovereign {
  deny jurisdictions = [ us ]
}

workload clinic {
  data records {
    sensitivity = S3
    keys        = customer       // every service using this data needs customer-held keys
  }
}
```

### Offers

An **offer** describes somewhere software *could* run: who provides it, where it is, how trusted it is, what it offers and what it costs. Bailiwick checks workloads against offers to decide what is allowed.

```bwk
offer example-fra-1 {
  vendor    = examplecloud
  country   = de
  trust     = T3
  classes   = [ backend-api, database ]
  price     = $0.02 / vcpu_hour
  resources { vcpu = 16  mem = 64 GB  disk = 400 GB  gpu = 0 }
}
```

---

## The command-line tool

The `bailiwick` binary the extension uses is also a command-line tool.

```sh
bailiwick check   app.bwk [--policy org.bwk]     # check a file, as the editor does
bailiwick fmt     app.bwk --write                # format in place
bailiwick explain app.bwk --catalog offers.bwk   # which offers are allowed, and why not
bailiwick import  terraform main.tf              # also: vendor-u, compose, k8s
bailiwick ir      app.bwk [--proto]              # the canonical form, as JSON or Protobuf
```

`explain` shows the reason for every decision:

```text
workload shop
  service api [backend-api]  residency=eu sensitivity=S3 min_trust=T3
    requires: vcpu>=2 mem>=4GB
    feasible: b-fr, c-fr  (2 vendor(s))
    excluded a-de: trust T2 is below the required T3
    excluded e-us-va: country `us` is outside residency `eu`
    requires >= 2 vendor(s)
  => deployable
```

And a mistake reads like this:

```text
app.bwk:3:18: error: `GB` is a data-size unit; durations use ms, s, m, h or d
```

`import` turns an existing `vendor-u.toml`, `docker-compose.yml`, Kubernetes manifest or Terraform file into a Bailiwick starting point. Every requirement it infers is marked `TODO: confirm`, with the line it came from, because only you know which of your current choices are actually requirements.

---

## Settings

| Setting | Default | |
|---|---|---|
| `bwk.serverPath` | `bailiwick` | Use your own `bailiwick` binary instead of the bundled one. |

---

## License

Copyright 2026 Helocore Corp. Written by Phantombyter.

Licensed under the [Apache License 2.0](https://github.com/Helocorecorp/bailiwick/blob/main/LICENSE).

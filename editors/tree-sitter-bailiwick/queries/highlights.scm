; Highlight queries for tree-sitter-bailiwick.

(comment) @comment

(string) @string
(money) @number
(number) @number
(duration_unit) @type
(data_unit) @type
(sensitivity_level) @constant
(trust_level) @constant
(boolean) @boolean

[
  "workload" "offer" "policy" "import"
  "data" "service" "job" "resources" "redundancy"
] @keyword

[
  "profile" "budget" "value_of" "latency" "per" "requests"
  "image" "sensitivity" "residency" "slo" "p95" "depends_on"
  "cache" "for" "never" "requires" "deadline" "checkpointable"
  "vendor" "country" "subdivision" "trust" "classes" "price" "features"
  "min_trust" "deny" "vendors" "countries" "jurisdictions" "keys" "customer" "from"
  "vcpu" "mem" "disk" "gpu"
] @property

(workload name: (identifier) @type)
(policy name: (identifier) @type)
(offer id: (identifier) @type)
(data_block name: (identifier) @variable)
(service name: (identifier) @function)
(job name: (identifier) @function)
(service class: (identifier) @type.builtin)
(job class: (identifier) @type.builtin)
(data_field region: (identifier) @constant.builtin)
(service_field region: (identifier) @constant.builtin)
(job_field region: (identifier) @constant.builtin)
(policy_field region: (identifier) @constant.builtin)
(service_field target: (identifier) @variable)

[ "=" "<=" ">=" "/" ":" ] @operator
[ "{" "}" "[" "]" ] @punctuation.bracket
"," @punctuation.delimiter

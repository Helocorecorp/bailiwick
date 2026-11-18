// Tree-sitter grammar for the Bailiwick (BWK), v0.
// Mirrors spec/grammar.ebnf. Generate with `tree-sitter generate`; the
// reference toolchain does not depend on this — it is for editors that
// use tree-sitter for highlighting, folding and structural navigation
// (Neovim, Helix, Zed, ...).

module.exports = grammar({
  name: "bailiwick",

  extras: ($) => [/\s/, $.comment],

  word: ($) => $.identifier,

  rules: {
    source_file: ($) => repeat($._item),

    _item: ($) => choice($.import, $.policy, $.workload, $.offer),

    comment: ($) => token(seq("//", /.*/)),

    // --- top level -----------------------------------------------------------

    import: ($) => seq("import", field("path", $.string)),

    policy: ($) =>
      seq("policy", field("name", $.identifier), "{", repeat($.policy_field), "}"),

    policy_field: ($) =>
      choice(
        seq("residency", "=", field("region", $.identifier)),
        seq("min_trust", "=", $.trust_level),
        seq("deny", choice("vendors", "countries", "jurisdictions"), "=", $.identifier_list),
        $.keys,
        seq("budget", "<=", $.money, "/", $.identifier)
      ),

    workload: ($) =>
      seq("workload", field("name", $.identifier), "{", repeat($._workload_item), "}"),

    _workload_item: ($) =>
      choice($.profile, $.budget, $.value_of, $.data_block, $.service, $.job),

    profile: ($) => seq("profile", "=", $.identifier),
    budget: ($) => seq("budget", "=", $.money, "/", $.identifier),
    value_of: ($) =>
      seq("value_of", "latency", "=", $.money, "per", $.duration, "per", $.number, "requests"),

    data_block: ($) =>
      seq("data", field("name", $.identifier), "{", repeat($.data_field), "}"),

    data_field: ($) =>
      choice(
        seq("sensitivity", "=", $.sensitivity_level),
        seq("residency", "=", field("region", $.identifier)),
        $.keys
      ),

    service: ($) =>
      seq(
        "service",
        field("name", $.identifier),
        ":",
        field("class", $.identifier),
        "{",
        repeat($.service_field),
        "}"
      ),

    service_field: ($) =>
      choice(
        seq("image", "=", $.string),
        seq("sensitivity", "=", $.sensitivity_level),
        seq("residency", "=", field("region", $.identifier)),
        seq("slo", ".", "p95", "<=", $.duration, optional(seq("from", choice($.identifier, $.identifier_list)))),
        seq("redundancy", "{", "vendors", ">=", $.number, "}"),
        seq("depends_on", "=", field("target", $.identifier)),
        $.resources_required,
        $.value_of,
        $.cache_rule,
        seq("requires", "=", $.identifier_list),
        $.keys
      ),

    cache_rule: ($) =>
      seq("cache", field("path", $.string), choice(seq("for", $.duration), "never")),

    job: ($) =>
      seq(
        "job",
        field("name", $.identifier),
        ":",
        field("class", $.identifier),
        "{",
        repeat($.job_field),
        "}"
      ),

    job_field: ($) =>
      choice(
        seq("image", "=", $.string),
        seq("deadline", "=", $.duration),
        seq("checkpointable", "=", $.boolean),
        seq("sensitivity", "=", $.sensitivity_level),
        seq("residency", "=", field("region", $.identifier)),
        $.resources_required,
        $.keys
      ),

    offer: ($) =>
      seq("offer", field("id", $.identifier), "{", repeat($.offer_field), "}"),

    offer_field: ($) =>
      choice(
        seq("vendor", "=", $.identifier),
        seq("country", "=", $.identifier),
        seq("subdivision", "=", $.identifier),
        seq("trust", "=", $.trust_level),
        seq("classes", "=", $.identifier_list),
        seq("price", "=", $.money, "/", $.identifier),
        $.resources_provided,
        seq("features", "=", $.identifier_list),
        seq("jurisdictions", "=", $.identifier_list),
        seq("latency", "{", repeat(seq("p95", "from", $.identifier, "=", $.duration)), "}"),
        $.keys
      ),

    // Who holds the encryption keys.
    keys: ($) => seq("keys", "=", choice("customer", "vendor")),

    // --- resources ------------------------------------------------------------

    resources_required: ($) =>
      seq("resources", "{", repeat($.resource_min), "}"),
    resource_min: ($) =>
      choice(
        seq(choice("vcpu", "gpu"), ">=", $.number),
        seq(choice("mem", "disk"), ">=", $.data_size)
      ),

    resources_provided: ($) =>
      seq("resources", "{", repeat($.resource_value), "}"),
    resource_value: ($) =>
      choice(
        seq(choice("vcpu", "gpu"), "=", $.number),
        seq(choice("mem", "disk"), "=", $.data_size)
      ),

    // --- values -----------------------------------------------------------------

    identifier_list: ($) =>
      seq("[", optional(seq($.identifier, repeat(seq(",", $.identifier)), optional(","))), "]"),

    money: ($) => seq("$", $.number),
    duration: ($) => seq($.number, $.duration_unit),
    duration_unit: ($) => choice("ms", "s", "m", "h", "d"),
    data_size: ($) => seq($.number, $.data_unit),
    data_unit: ($) => choice("MB", "GB", "TB"),

    sensitivity_level: ($) => /S[0-3]/,
    trust_level: ($) => /T[0-3]/,
    boolean: ($) => choice("true", "false"),

    number: ($) => /[0-9]+(\.[0-9]+)?/,
    string: ($) => seq('"', repeat(choice(/[^"\\\n]+/, /\\[\\"nt]/)), '"'),
    identifier: ($) => /[A-Za-z_][A-Za-z0-9_]*(-[A-Za-z0-9]+)*/,
  },
});

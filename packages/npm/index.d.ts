export interface Result {
  /** 0 success, 1 diagnostics or a plan that cannot deploy, 2 usage. */
  exit: number;
  stdout: string;
  stderr: string;
}

export interface Options {
  /** Rule packs by name (`gdpr`, `hipaa`, ...) or policy files. */
  policies?: string[];
  /** A catalog of offers; the built-in open catalog when omitted. */
  catalog?: string;
  /** For `check-plan`: Terraform plans (JSON) and/or Kubernetes manifests. */
  plans?: string[];
  /** For `check-plan` with Kubernetes: whose cluster (aws, gcp, azure, ...). */
  vendor?: string;
  /** For `certify`: placements, `unit=offer[,offer...]`. */
  place?: string[];
  /** For `prove`: policies or rule packs to check the program already satisfies. */
  implies?: string[];
  /** For `prove`: an earlier version; proves this one allows nothing it did not. */
  baseline?: string;
  format?: "text" | "json" | "markdown";
  /** How to read files; `fs.readFileSync` by default. */
  read?: (path: string) => string;
}

export function run(command: "check" | "explain" | "fmt" | "certify" | "check-plan" | "prove", file: string, opts?: Options): Result;

export function evaluate(source: string): {
  diagnostics: { line: number; col: number; severity: "error" | "warning"; message: string }[];
  formatted: string;
  report: string;
  plan: unknown;
  certificate: unknown;
  catalog: "own" | "bundled";
  offers: number;
};

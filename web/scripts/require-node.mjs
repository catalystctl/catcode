import { runtimeRequirementError } from "./runtime-requirements.mjs";

const error = runtimeRequirementError(process.versions);
if (error) {
  console.error(error);
  process.exit(1);
}

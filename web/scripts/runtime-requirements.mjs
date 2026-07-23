export const MINIMUM_NODE = [22, 13, 0];

export function nodeVersionIsTooOld(version, minimum = MINIMUM_NODE) {
  const current = String(version ?? "").split(".").map(Number);
  if (
    current.length < 2 ||
    current.some((part) => !Number.isFinite(part))
  ) {
    return true;
  }
  return (
    current[0] < minimum[0] ||
    (current[0] === minimum[0] && current[1] < minimum[1]) ||
    (current[0] === minimum[0] &&
      current[1] === minimum[1] &&
      (current[2] ?? 0) < minimum[2])
  );
}

export function runtimeRequirementError(versions) {
  if (versions?.bun) {
    return "Catalyst Code web build and server require Node.js 22.13+; Bun's node shim is not supported because the application uses node:sqlite.";
  }
  if (nodeVersionIsTooOld(versions?.node)) {
    return `Catalyst Code web build and server require Node.js 22.13+ (found ${versions?.node ?? "unknown"}).`;
  }
  return null;
}

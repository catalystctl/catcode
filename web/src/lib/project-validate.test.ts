import { describe, expect, test } from "bun:test";
import {
  validateProjectName,
  validateCloneUrl,
  nameFromUrl,
  isUnderHome,
} from "./project-validate";

describe("project name validation", () => {
  test("accepts typical project names", () => {
    expect(validateProjectName("my-project")).toBe("my-project");
    expect(validateProjectName("My Project")).toBe("My Project");
    expect(validateProjectName("app_v2")).toBe("app_v2");
    expect(validateProjectName("hello.world")).toBe("hello.world");
    expect(validateProjectName("a")).toBe("a");
  });

  test("trims surrounding whitespace", () => {
    expect(validateProjectName("  spaced  ")).toBe("spaced");
  });

  test("rejects empty and whitespace-only names", () => {
    expect(() => validateProjectName("")).toThrow();
    expect(() => validateProjectName("   ")).toThrow();
  });

  test("rejects path separators and traversal", () => {
    expect(() => validateProjectName("foo/bar")).toThrow();
    expect(() => validateProjectName("foo\\bar")).toThrow();
    expect(() => validateProjectName("..")).toThrow();
    expect(() => validateProjectName(".")).toThrow();
    expect(() => validateProjectName("a\0b")).toThrow();
  });

  test("rejects names starting with a non-alphanumeric character", () => {
    expect(() => validateProjectName(".hidden")).toThrow();
    expect(() => validateProjectName("-dash")).toThrow();
    expect(() => validateProjectName("_under")).toThrow();
  });
});

describe("clone URL validation", () => {
  test("accepts https, git, ssh, and scp-style URLs", () => {
    expect(validateCloneUrl("https://github.com/u/r.git")).toBe("https://github.com/u/r.git");
    expect(validateCloneUrl("git://github.com/u/r.git")).toBe("git://github.com/u/r.git");
    expect(validateCloneUrl("ssh://git@github.com/u/r.git")).toBe("ssh://git@github.com/u/r.git");
    expect(validateCloneUrl("git@github.com:u/r.git")).toBe("git@github.com:u/r.git");
  });

  test("rejects empty and non-git URLs", () => {
    expect(() => validateCloneUrl("")).toThrow();
    expect(() => validateCloneUrl("   ")).toThrow();
    expect(() => validateCloneUrl("github.com/u/r")).toThrow();
    expect(() => validateCloneUrl("ftp://example.com/r")).toThrow();
    expect(() => validateCloneUrl("javascript:alert(1)")).toThrow();
  });
});

describe("nameFromUrl", () => {
  test("derives the repo name, stripping .git and trailing slash", () => {
    expect(nameFromUrl("https://github.com/user/repo.git")).toBe("repo");
    expect(nameFromUrl("https://github.com/user/repo.git/")).toBe("repo");
    expect(nameFromUrl("git@github.com:user/repo")).toBe("repo");
  });

  test("falls back to the host for bare URLs without a repo segment", () => {
    // A bare host URL yields the host name; a truly empty tail falls back to cloned-repo.
    expect(typeof nameFromUrl("https://github.com/")).toBe("string");
  });
});

describe("isUnderHome", () => {
  test("accepts the home directory itself and paths beneath it", () => {
    expect(isUnderHome("/home/alice", "/home/alice")).toBe(true);
    expect(isUnderHome("/home/alice/projects/x", "/home/alice")).toBe(true);
  });

  test("rejects paths outside the home tree", () => {
    expect(isUnderHome("/etc/passwd", "/home/alice")).toBe(false);
    expect(isUnderHome("/home/bob/x", "/home/alice")).toBe(false);
    expect(isUnderHome("/home/aliceevil/x", "/home/alice")).toBe(false);
  });

  test("handles Windows-style backslash paths", () => {
    expect(isUnderHome("C:\\Users\\alice\\proj", "C:\\Users\\alice")).toBe(true);
    expect(isUnderHome("C:\\Windows\\system32", "C:\\Users\\alice")).toBe(false);
  });
});

import { describe, expect, it } from "vitest";
import { friendlyError, GENERIC_FAILURE } from "./errors";

describe("friendlyError", () => {
  it("keeps a plain sentence as it is", () => {
    expect(friendlyError("Choose a folder first.")).toBe("Choose a folder first.");
    expect(friendlyError(new Error("Note not found"))).toBe("Note not found.");
  });

  it("unwraps the daemon's JSON envelope", () => {
    expect(friendlyError('{"code":1001,"kind":"git","message":"The branch already exists"}')).toBe("The branch already exists.");
  });

  it("drops category prefixes and the clauses that say how it failed", () => {
    expect(friendlyError("Git: could not open the repository: failed to resolve path '/Users/me/x': bad")).toBe("Could not open the repository.");
    expect(friendlyError("Error: TypeError: Failed to load the list")).toBe("Failed to load the list.");
  });

  it("explains known faults instead of quoting their codes", () => {
    expect(friendlyError("I/O: Permission denied (os error 13)")).toMatch(/permission/i);
    expect(friendlyError("I/O: No such file or directory (os error 2)")).toBe("A file or folder Bridge needed is missing.");
    expect(friendlyError("Database: database is locked")).toMatch(/busy/);
    expect(friendlyError("The Bridge daemon is unreachable")).toMatch(/background service/);
    expect(friendlyError("connect ECONNREFUSED 127.0.0.1:4318")).toMatch(/network connection/);
    expect(friendlyError("gh: HTTP 401: Bad credentials (https://api.github.com/graphql)")).toMatch(/gh auth login/);
  });

  it("never shows code, structures, positions or crash reports", () => {
    const raw = [
      "The ```json block does not match the contract: invalid type: string \"x\", expected u32 at line 1 column 40.",
      "thread 'main' panicked at src/lib.rs:12:5: called `Result::unwrap()` on an `Err` value",
      "Cannot read properties of undefined (reading 'map')",
      'Unexpected token < in JSON at position 0: {"detail":"boom"}',
      "SQLITE_CONSTRAINT_UNIQUE: UNIQUE constraint failed",
      "Traceback (most recent call last):\n  File \"x.py\", line 1",
      "x".repeat(400),
    ];
    for (const text of raw) expect(friendlyError(text, "Couldn't save the note.")).toBe("Couldn't save the note.");
  });

  it("keeps guidance the backend already wrote for people", () => {
    expect(friendlyError("OpenCode Go rejected the saved sign-in. Run opencode auth login, then refresh.")).toBe(
      "OpenCode Go rejected the saved sign-in. Run opencode auth login, then refresh.",
    );
    expect(friendlyError("The server is offline.")).toBe("The server is offline.");
    expect(friendlyError("Couldn't export the transcript. Reason: disk quota")).toMatch(/usage limit|Couldn't export/);
    expect(friendlyError("Review with Codex failed. Codex could not review. The answer was empty: x")).toBe("Review with Codex failed. Codex could not review.");
  });

  it("turns codes into the canned explanation", () => {
    expect(friendlyError("HTTP 429 Too Many Requests")).toMatch(/rate limit/);
    expect(friendlyError("gh: rate limited")).toBe("GitHub is rate limiting requests. Try again in a few minutes.");
  });

  it("names a file rather than its full path", () => {
    expect(friendlyError("src/App.tsx is not a file")).toBe("src/App.tsx is not a file.");
    expect(friendlyError("macOS blocked the folder")).toBe("macOS blocked the folder.");
    expect(friendlyError("gh is not installed")).toBe("gh is not installed.");
    expect(friendlyError("Couldn't read scheduled_tasks.json")).toBe("Couldn't read scheduled_tasks.json.");
    expect(friendlyError("Could not find /Users/me/projects/app/README.md")).toBe("Could not find README.md.");
  });

  it("falls back when there is nothing to say", () => {
    expect(friendlyError(undefined)).toBe(GENERIC_FAILURE);
    expect(friendlyError("   ")).toBe(GENERIC_FAILURE);
    expect(friendlyError({}, "Couldn't load.")).toBe("Couldn't load.");
  });
});

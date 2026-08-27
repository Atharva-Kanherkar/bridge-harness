import { describe, expect, it } from "vitest";
import { createBridgeQueryClient, queryKeys } from "./queryClient";

describe("Bridge query client", () => {
  it("uses stable keys for shared server state", () => {
    expect(queryKeys).toEqual({
      health: ["health"],
      modelSetup: ["model-setup"],
      workBoard: ["work-board"],
    });
  });

  it("does not retry local Tauri calls or refetch on generic window focus", () => {
    const options = createBridgeQueryClient().getDefaultOptions().queries;

    expect(options?.retry).toBe(false);
    expect(options?.refetchOnWindowFocus).toBe(false);
    expect(options?.staleTime).toBe(30_000);
  });
});

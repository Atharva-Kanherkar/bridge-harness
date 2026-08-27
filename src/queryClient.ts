import { QueryClient } from "@tanstack/react-query";

export const queryKeys = {
  health: ["health"] as const,
  modelSetup: ["model-setup"] as const,
  workBoard: ["work-board"] as const,
};

export function createBridgeQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        refetchOnWindowFocus: false,
        retry: false,
        staleTime: 30_000,
      },
    },
  });
}

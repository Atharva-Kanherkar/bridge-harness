import { useCallback, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { bridgeApi } from "./api";
import { queryKeys } from "./queryClient";
import type { ModelSetupState } from "./types";

export function useBridgeServerState() {
  const queryClient = useQueryClient();
  const health = useQuery({ queryKey: queryKeys.health, queryFn: () => bridgeApi.health() });
  const modelSetup = useQuery({ queryKey: queryKeys.modelSetup, queryFn: () => bridgeApi.modelSetup() });
  const [followedRunId, followWorkBriefing] = useState<string>();
  const workBoard = useQuery({
    queryKey: queryKeys.workBoard,
    queryFn: async () => {
      const board = await bridgeApi.workBoard();
      // A successful read begun after the receipt hands tracking over to the
      // stored running state. Do not clear a newer receipt from an older read.
      if (followedRunId) {
        followWorkBriefing(current => current === followedRunId ? undefined : current);
      }
      return board;
    },
    // Follow accepted receipts even if the first read fails and leaves old data.
    // Observed running boards also opt in, including runs started elsewhere.
    enabled: query => !!followedRunId || query.state.data?.suggestions.state === "running",
    refetchInterval: query => followedRunId || query.state.data?.suggestions.state === "running" ? 2_000 : false,
    refetchIntervalInBackground: true,
  });
  const acceptModelSetup = useCallback((setup: ModelSetupState) => {
    queryClient.setQueryData(queryKeys.modelSetup, setup);
  }, [queryClient]);
  const invalidateHealth = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.health });
  }, [queryClient]);

  return {
    health: health.data,
    healthError: health.error,
    modelSetup: modelSetup.data,
    modelSetupError: modelSetup.error,
    workBoard: workBoard.data,
    workBoardQueryError: workBoard.error,
    refetchWorkBoard: workBoard.refetch,
    followWorkBriefing,
    acceptModelSetup,
    invalidateHealth,
  };
}

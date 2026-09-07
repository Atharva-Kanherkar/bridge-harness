import { useCallback } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { bridgeApi } from "./api";
import { queryKeys } from "./queryClient";
import type { ModelSetupState } from "./types";

export function useBridgeServerState() {
  const queryClient = useQueryClient();
  const health = useQuery({ queryKey: queryKeys.health, queryFn: () => bridgeApi.health() });
  const modelSetup = useQuery({ queryKey: queryKeys.modelSetup, queryFn: () => bridgeApi.modelSetup() });
  const workBoard = useQuery({
    queryKey: queryKeys.workBoard,
    queryFn: () => bridgeApi.workBoard(),
    enabled: false,
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
    acceptModelSetup,
    invalidateHealth,
  };
}

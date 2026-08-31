import { create } from "zustand";

export type AppModal = "workspace" | "orchestrator" | "router" | "memory" | "memory-core";

type UiState = {
  modal: AppModal | null;
  openModal: (modal: AppModal) => void;
  closeModal: () => void;
};

export const useUiStore = create<UiState>(set => ({
  modal: null,
  openModal: modal => set({ modal }),
  closeModal: () => set({ modal: null }),
}));

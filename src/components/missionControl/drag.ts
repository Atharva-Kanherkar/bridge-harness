export const TILE_DRAG = "application/x-bridge-mission-tile";
export const SIDEBAR_CHAT_DRAG = "application/x-bridge-sidebar-chat";

export function isChatDrag(data: Pick<DataTransfer, "types">): boolean {
  return data.types.includes(TILE_DRAG) || data.types.includes(SIDEBAR_CHAT_DRAG);
}

export function readChatDrag(data: Pick<DataTransfer, "getData">): { id: string; fromSidebar: boolean } {
  const sidebarId = data.getData(SIDEBAR_CHAT_DRAG);
  return { id: sidebarId || data.getData(TILE_DRAG), fromSidebar: !!sidebarId };
}

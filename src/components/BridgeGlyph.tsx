import { BRIDGE_ICON_PATH } from "../brand/bridgeIcon";

export function BridgeGlyph({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden="true">
      <path d={BRIDGE_ICON_PATH} fill="currentColor" fillRule="evenodd" />
    </svg>
  );
}

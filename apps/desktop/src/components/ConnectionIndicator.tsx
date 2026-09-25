import type { GatewayStatus } from '../lib/gateway';

// Presence shapes from the design language: filled dot when live, pulsing
// amber while (re)connecting, hollow ring when offline. The word always
// accompanies the colour.
const LABEL: Record<GatewayStatus, string> = {
  connected: 'Online',
  connecting: 'Connecting…',
  reconnecting: 'Reconnecting…',
  disconnected: 'Offline',
};

export default function ConnectionIndicator({
  status,
}: {
  status: GatewayStatus;
}) {
  return (
    <span className="status-chip" title={`gateway: ${status}`}>
      <span aria-hidden className={`status-dot status-dot--${status}`} />
      {LABEL[status]}
      <span className="status-chip-scope">gateway</span>
    </span>
  );
}

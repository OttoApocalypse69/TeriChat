import type { GatewayStatus } from '../lib/gateway';

const DOT: Record<GatewayStatus, string> = {
  disconnected: 'bg-zinc-500',
  connecting: 'bg-amber-400',
  connected: 'bg-emerald-400',
  reconnecting: 'bg-amber-400 animate-pulse',
};

export default function ConnectionIndicator({
  status,
}: {
  status: GatewayStatus;
}) {
  return (
    <span
      className="inline-flex items-center gap-1.5 text-xs text-zinc-300"
      title={`gateway: ${status}`}
    >
      <span className={`inline-block h-2 w-2 rounded-full ${DOT[status]}`} />
      {status}
    </span>
  );
}

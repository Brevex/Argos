const BYTE_UNITS = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'];

export const formatBytes = (bytes: number): string => {
  if (bytes <= 0) return '0 B';
  const exp = Math.min(
    BYTE_UNITS.length - 1,
    Math.floor(Math.log(bytes) / Math.log(1024)),
  );
  const value = bytes / Math.pow(1024, exp);
  const formatted = value >= 100 ? value.toFixed(0) : value.toFixed(1);
  return `${formatted} ${BYTE_UNITS[exp]}`;
};

export const formatCount = (n: number): string =>
  new Intl.NumberFormat('en-US').format(n);

export const formatPercent = (ratio: number): string =>
  `${(Math.min(1, Math.max(0, ratio)) * 100).toFixed(1)}%`;

export const formatDuration = (ms: number): string => {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => n.toString().padStart(2, '0');
  return `${pad(h)}:${pad(m)}:${pad(s)}`;
};

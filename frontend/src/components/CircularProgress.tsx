interface CircularProgressProps {
  ratio: number | null;
  size?: number;
  stroke?: number;
}

export default function CircularProgress(props: CircularProgressProps) {
  const size = props.size ?? 96;
  const stroke = props.stroke ?? 7;
  const radius = (size - stroke) / 2;
  const circumference = 2 * Math.PI * radius;
  const indeterminate = props.ratio === null;
  const clamped = () => Math.min(1, Math.max(0, props.ratio ?? 0));
  const offset = () => circumference * (1 - clamped());
  const percent = () => Math.round(clamped() * 100);

  return (
    <div
      class={`circular ${indeterminate ? 'indeterminate' : ''}`}
      style={{ width: `${size}px`, height: `${size}px` }}
    >
      <svg viewBox={`0 0 ${size} ${size}`} width={size} height={size}>
        <defs>
          <linearGradient id="ringGradient" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stop-color="#7aa9ff" />
            <stop offset="100%" stop-color="#4f7dff" />
          </linearGradient>
        </defs>
        <circle
          class="circular-track"
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke-width={stroke}
        />
        <circle
          class="circular-fill"
          cx={size / 2}
          cy={size / 2}
          r={radius}
          fill="none"
          stroke="url(#ringGradient)"
          stroke-width={stroke}
          stroke-linecap="round"
          stroke-dasharray={String(circumference)}
          stroke-dashoffset={String(indeterminate ? circumference * 0.7 : offset())}
          transform={`rotate(-90 ${size / 2} ${size / 2})`}
        />
      </svg>
      <div class="circular-label">
        {indeterminate ? (
          <span class="circular-dots" aria-hidden="true">
            <span /> <span /> <span />
          </span>
        ) : (
          <span class="circular-readout">
            <span class="circular-value">{percent()}</span>
            <span class="circular-suffix">%</span>
          </span>
        )}
      </div>
    </div>
  );
}

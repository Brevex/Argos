import type { JSX } from 'solid-js';

export type MetricTone = 'green' | 'purple' | 'cyan' | 'orange';

interface MetricCardProps {
  icon: JSX.Element;
  value: string;
  label: string;
  tone: MetricTone;
}

export default function MetricCard(props: MetricCardProps) {
  return (
    <div class="metric-card">
      <span class={`metric-card-icon ${props.tone}`} aria-hidden="true">
        {props.icon}
      </span>
      <span class="metric-card-text">
        <span class="metric-card-value">{props.value}</span>
        <span class="metric-card-label">{props.label}</span>
      </span>
    </div>
  );
}

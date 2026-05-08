import type { JSX } from 'solid-js';

interface GlassProps {
  children: JSX.Element;
  class?: string;
}

export default function Glass(props: GlassProps) {
  return (
    <section class={`glass ${props.class ?? ''}`}>
      <div class="glass-body">{props.children}</div>
    </section>
  );
}

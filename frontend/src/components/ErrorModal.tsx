import { Show, onCleanup, onMount } from 'solid-js';
import { Portal } from 'solid-js/web';
import { AlertIcon } from './icons';

interface ErrorModalProps {
  message: string | null;
  onClose: () => void;
}

export default function ErrorModal(props: ErrorModalProps) {
  const handleKey = (event: KeyboardEvent) => {
    if (event.key === 'Escape' && props.message) props.onClose();
  };

  onMount(() => document.addEventListener('keydown', handleKey));
  onCleanup(() => document.removeEventListener('keydown', handleKey));

  const handleBackdropClick = (event: MouseEvent) => {
    if (event.target === event.currentTarget) props.onClose();
  };

  return (
    <Show when={props.message}>
      <Portal>
        <div
          class="modal-backdrop"
          role="presentation"
          onClick={handleBackdropClick}
        >
          <div
            class="modal glass"
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="error-modal-title"
          >
            <span class="modal-icon" aria-hidden="true">
              <AlertIcon />
            </span>
            <div class="modal-body">
              <h3 class="modal-title" id="error-modal-title">
                Something went wrong
              </h3>
              <p class="modal-message">{props.message}</p>
            </div>
            <div class="modal-actions">
              <button
                type="button"
                class="btn primary"
                onClick={props.onClose}
                autofocus
              >
                Dismiss
              </button>
            </div>
          </div>
        </div>
      </Portal>
    </Show>
  );
}

/** Mounts the shell. */
import { mount } from 'svelte';

import Shell from './layout/Shell.svelte';
import './app.css';

mount(Shell, { target: document.getElementById('app')! });

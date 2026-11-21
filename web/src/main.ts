import { mount } from 'svelte';
import App from './App.svelte';
import './console.css';

const target = document.getElementById('app');
if (!target) throw new Error('Console mount point is missing.');
mount(App, { target });

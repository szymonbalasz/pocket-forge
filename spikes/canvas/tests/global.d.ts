import type { bench } from '../src/bench';
declare global { interface Window { bench: typeof bench } }

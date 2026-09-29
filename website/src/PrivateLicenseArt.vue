<template>
  <svg class="private-license-art" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
    <defs>
      <linearGradient id="private-license-glass" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="#72c9ef" stop-opacity=".52" />
        <stop offset=".42" stop-color="#285b79" stop-opacity=".78" />
        <stop offset="1" stop-color="#10253f" stop-opacity=".94" />
      </linearGradient>
      <filter id="private-license-glow" x="-40%" y="-40%" width="180%" height="180%">
        <feGaussianBlur stdDeviation="2.2" result="blur" />
        <feMerge><feMergeNode in="blur" /><feMergeNode in="SourceGraphic" /></feMerge>
      </filter>
    </defs>
    <path d="M10 8h54l25 25v56l-7 7H10L4 90V14Z" fill="#061221" stroke="#1c4b69" stroke-width="2" />
    <path d="M9 2h55l25 25v57l-7 7H9L4 86V8Z" fill="url(#private-license-glass)" stroke="#d1f4ff" stroke-width="1.7" filter="url(#private-license-glow)" />
    <path d="M64 2v25h25" fill="#4a86a3" fill-opacity=".58" stroke="#d1f4ff" stroke-width="1.6" />
    <path d="M10 6h50" stroke="#fff" stroke-opacity=".52" stroke-width="1.2" stroke-linecap="round" />
    <path d="M22 39h47M22 50h38M22 61h31" stroke="#5b7d96" stroke-width="4.5" stroke-linecap="round" opacity=".52" />
    <path d="m73 61 11 4 8 10-2 12-8 9-12 2-10-8-3-12 4-10Z" fill="#2a7890" stroke="#a5fff4" stroke-width="1.4" filter="url(#private-license-glow)" />
    <path d="m66 79 6 6 10-13" fill="none" stroke="#a6fff4" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" />
  </svg>
</template>

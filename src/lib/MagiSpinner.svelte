<!--
  Magi's mark, thinking.

  The same three nodes as the tray icon — Melchior, Balthasar and Casper, the
  three Magi — laid out on a circle at -90°, 30° and 150°, apex up. Reusing the
  mark rather than a generic spinner means the waiting state says *Magi* is
  working, not merely that something is.

  Two nested groups, because one element can hold only one `transform`. The outer
  rotates, the inner breathes, and each keeps its own period — so the two motions
  drift in and out of phase instead of locking into a single mechanical loop.

  The breathing is a scale on the group, which reads as the nodes moving toward
  and away from the centre: they sit on radii, so scaling moves each one along
  its own radius. Three separate translations would look identical and take three
  times the code.
-->
<script lang="ts">
  let { label = "Magi is thinking" }: { label?: string } = $props();
</script>

<svg viewBox="0 0 24 24" role="img" aria-label={label}>
  <g class="orbit">
    <g class="breathe">
      <circle cx="12" cy="5" r="2.6" />
      <circle cx="18.06" cy="15.5" r="2.6" />
      <circle cx="5.94" cy="15.5" r="2.6" />
    </g>
  </g>
</svg>

<style>
  svg {
    height: 22px;
    width: 22px;
  }

  circle {
    fill: currentColor;
  }

  /* Both groups turn about 12,12 — the circle the nodes sit on.
     `transform-origin: center` would resolve to the bounding box's centre, and
     with the apex up that box is taller above the middle node row than below
     it, putting its centre at y≈10.25. Rotating about that point would make the
     mark wobble rather than spin. The centre has to be stated outright. */
  .orbit,
  .breathe {
    transform-origin: 12px 12px;
  }

  .orbit {
    animation: orbit 2.4s linear infinite;
  }

  .breathe {
    animation: breathe 1.5s ease-in-out infinite;
  }

  @keyframes orbit {
    to {
      transform: rotate(360deg);
    }
  }

  @keyframes breathe {
    0%,
    100% {
      transform: scale(1);
    }
    50% {
      transform: scale(0.62);
    }
  }

  /* Reduced motion is a request to stop moving, not to hide the indicator: the
     user still needs to know an answer is coming. The mark holds still and
     fades instead. */
  @media (prefers-reduced-motion: reduce) {
    .orbit {
      animation: none;
    }

    .breathe {
      animation: fade 1.8s ease-in-out infinite;
    }

    @keyframes fade {
      50% {
        opacity: 0.35;
      }
    }
  }
</style>

(() => {
  const container = document.getElementById("demo-player");
  const status = document.getElementById("player-status");
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  if (!window.AsciinemaPlayer) {
    status.innerHTML = 'The player could not load. <a href="uhm-demo.cast">Download the asciicast</a> or <a href="uhm-demo.gif">watch the GIF</a>.';
    return;
  }

  window.AsciinemaPlayer.create("uhm-demo.cast", container, {
    autoplay: !reducedMotion,
    loop: !reducedMotion,
    preload: true,
    fit: "width",
    theme: "nord",
  });
})();

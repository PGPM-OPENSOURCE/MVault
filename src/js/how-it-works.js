  // Toggle "How It Works" page styles (and later, optional matrix)
  function applyHowItWorksTheme(theme) {
    // 1) Flip stylesheets
    theme = MemoroTheme.flip(theme);
    const isCypher = (theme === 'cypherpunk');

    // 2) Body attribute (safe for CSS hooks)
    const setBodyAttr = () => {
      if (document.body) {
        document.body.setAttribute('data-theme', theme);
      }
    };
    if (document.readyState === 'loading') {
      window.addEventListener('DOMContentLoaded', setBodyAttr, { once: true });
    } else {
      setBodyAttr();
    }

    // 3) Matrix background control (if implemented for HowItWorksMatrix)
    if (window.HowItWorksMatrix) {
      if (isCypher) HowItWorksMatrix.start();
      else HowItWorksMatrix.stop();
    }
  }

  // Read saved theme (theme-preflip.js already flipped the stylesheets before paint)
  (function initHowItWorksTheme(){
    window.__savedHowItWorksTheme = MemoroTheme.saved();
  })();

  applyHowItWorksTheme(window.__savedHowItWorksTheme || MemoroTheme.DEFAULT);

  const canvas = document.getElementById('matrixCanvas');
  const ctx = canvas.getContext('2d');
  canvas.width = window.innerWidth;
  canvas.height = window.innerHeight;
  const letters = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890@$%&'.split('');
  const fontSize = 14;
  const columns = Math.floor(canvas.width / fontSize);
  const drops = Array(columns).fill(0);

  (function(){
    const canvas = document.getElementById('matrixCanvas');
    const ctx = canvas.getContext('2d');

    function resize() {
      canvas.width = window.innerWidth;
      canvas.height = window.innerHeight;
    }
    resize();
    window.addEventListener('resize', resize);

    const letters = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890@$%&'.split('');
    const fontSize = 14;
    let columns = Math.floor(canvas.width / fontSize);
    let drops = Array(columns).fill(0);
    let timer = null;

    function drawMatrix() {
      // faint trail
      ctx.fillStyle = 'rgba(0, 0, 0, 0.15)';
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      ctx.fillStyle = '#00ff99';
      ctx.font = `${fontSize}px monospace`;

      for (let i = 0; i < drops.length; i++) {
        const text = letters[Math.floor(Math.random() * letters.length)];
        ctx.fillText(text, i * fontSize, drops[i] * fontSize);
        drops[i]++;
        if (drops[i] * fontSize > canvas.height && Math.random() > 0.975) {
          drops[i] = 0;
        }
      }
    }

    window.HowItWorksMatrix = {
      start() {
        resize();
        columns = Math.floor(canvas.width / fontSize);
        drops = Array(columns).fill(0);
        if (!timer) timer = setInterval(drawMatrix, 66);
        canvas.style.visibility = 'visible';
      },
      stop() {
        if (timer) {
          clearInterval(timer);
          timer = null;
        }
        // clear canvas so no ghosted frame appears
        ctx.clearRect(0, 0, canvas.width, canvas.height);
        canvas.style.visibility = 'hidden';
      }
    };
  })();

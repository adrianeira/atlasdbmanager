// Progresso da tela de abertura. O avanço é estimado (a abertura não tem etapas mensuráveis); quando a interface
// avisa que está pronta, o Rust chama `window.atlasSplashDone()` e a barra completa antes de a janela fechar.
(function () {
  var pt = /^pt/i.test(navigator.language || "");
  var stages = pt
    ? [[0, "Iniciando..."], [30, "Abrindo o banco local..."], [60, "Lendo as conexões..."], [85, "Carregando a interface..."]]
    : [[0, "Starting..."], [30, "Opening the local database..."], [60, "Reading connections..."], [85, "Loading the interface..."]];
  var doneText = pt ? "Pronto" : "Ready";
  var fill = document.getElementById("fill");
  var status = document.getElementById("status");
  var pct = 0;
  var done = false;

  function render() {
    fill.style.width = pct + "%";
    var text = stages[0][1];
    for (var i = 0; i < stages.length; i++) if (pct >= stages[i][0]) text = stages[i][1];
    status.textContent = done ? doneText : text;
  }

  // Aproxima-se de 92% sem nunca chegar: sempre parece andar, mesmo se a abertura demorar.
  var timer = setInterval(function () {
    if (done) return;
    pct += (92 - pct) * 0.06;
    render();
  }, 100);

  window.atlasSplashDone = function () {
    done = true;
    pct = 100;
    clearInterval(timer);
    render();
  };
  render();
})();

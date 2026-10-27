// Progresso da tela de abertura. O avanço é estimado (a abertura não tem etapas mensuráveis); quando a interface
// avisa que está pronta, o Rust chama `window.atlasSplashDone()` e a barra completa antes de a janela fechar.
(function () {
  // O idioma escolhido no app (Exibir > Idioma) fica no armazenamento local, que a splash compartilha com a janela principal;
  // sem escolha salva vale o idioma do sistema.
  var saved = null;
  try {
    var layout = JSON.parse(localStorage.getItem("atlas.layout.v1") || "null");
    saved = layout && layout.lang ? String(layout.lang) : null;
  } catch (e) {}
  var code = String(saved || navigator.language || "").toLowerCase();
  var pt = /^pt/.test(code);
  var es = /^es/.test(code);
  var stages = pt
    ? [[0, "Iniciando..."], [30, "Abrindo o banco local..."], [60, "Lendo as conexões..."], [85, "Carregando a interface..."]]
    : es
      ? [[0, "Iniciando..."], [30, "Abriendo la base de datos local..."], [60, "Leyendo las conexiones..."], [85, "Cargando la interfaz..."]]
      : [[0, "Starting..."], [30, "Opening the local database..."], [60, "Reading connections..."], [85, "Loading the interface..."]];
  var doneText = pt ? "Pronto" : es ? "Listo" : "Ready";
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

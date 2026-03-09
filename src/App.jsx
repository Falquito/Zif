import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/tauri';
import { listen } from '@tauri-apps/api/event';
import { open, save } from '@tauri-apps/api/dialog';
import './App.css';

function App() {
  const [level, setLevel] = useState(3);
  const [progress, setProgress] = useState(0);
  const [currentFile, setCurrentFile] = useState('');
  const [eta, setEta] = useState(0);
  const [status, setStatus] = useState('idle');
  const [destPath, setDestPath] = useState('');
  const [statusMessage, setStatusMessage] = useState('Listo');

  // Estados para el Deep Linking
  const [contexto, setContexto] = useState({ idInforme: null, usuario: 'Invitado', userId: null });

  useEffect(() => {
    // Escuchar el protocolo zif://
    listen('deep-link', (event) => {
      // El payload puede venir como string directo (args[1]) o como array de args.
      let urlString = "";

      if (Array.isArray(event.payload)) {
        // Si viene del Plugin Single Instance, es un array de argumentos.
        // Buscamos el que empieza con zif://
        const found = event.payload.find(s => s.startsWith && s.startsWith("zif://"));
        if (found) urlString = found;
      } else {
        // Si viene del setup inicial simple
        urlString = event.payload;
      }

      if (urlString) {
        console.log("Deep Link recibido:", urlString); // Para depurar
        const url = new URL(urlString);
        const params = new URLSearchParams(url.search);
        setContexto({
          idInforme: params.get('idInforme'),
          usuario: params.get('usuario') || 'Usuario',
          userId: params.get('userId')
        });

        // Feedback visual
        setStatusMessage(`Informe cargado: ${params.get('idInforme')}`);
      }
    });
    const unlisten = listen('progress', (event) => {
      const { percentage, current_file, eta_seconds, status: opStatus } = event.payload;
      setProgress(percentage);
      setCurrentFile(current_file);
      setEta(eta_seconds);

      if (opStatus === 'done') {
        setStatus('done');
        setStatusMessage('Operación terminada');
      } else {
        setStatus('working');
        setStatusMessage(level >= 22 ? '7-Zip: Comprimiendo...' : 'Facebook/Zstd: Comprimiendo...');
      }
    });
    return () => { unlisten.then(f => f()); };
  }, [level]);

  const handleAction = async (isFolder) => {
    const selected = await open({ directory: isFolder });
    if (!selected) return;
    const isUltra = level >= 22;
    const ext = isUltra ? '7z' : 'tar.zst';
    const savePath = await save({ defaultPath: `informe_${contexto.idInforme || 'archivo'}.${ext}` });
    if (!savePath) return;

    setDestPath(savePath);
    setStatus('working');
    await invoke('comprimir', { path: selected, destination: savePath, level: parseInt(level) });
  };

  const handleHash = async () => {
    try {
      setStatusMessage('Calculando Hash...');
      const hash = await invoke('calcular_y_enviar_hash', {
        path: destPath,
        idInforme: parseInt(contexto.idInforme),
        userId: parseInt(contexto.usuario)
      });
      alert(`Hash calculado y enviado: ${hash}`);
      setStatusMessage('Hash registrado con éxito');
    } catch (e) { alert(e); }
  };

  return (
    <div className="sevenzip-container">
      <div className="toolbar">
        <button className="tool-btn" onClick={() => handleAction(true)}>➕ Carpeta</button>
        <button className="tool-btn" onClick={() => handleAction(false)}>📄 Archivo</button>
        <button className="tool-btn" onClick={async () => {
          const p = await open();
          const d = await open({ directory: true });
          invoke('descomprimir', { tar_path: p, dest_folder: d });
        }}>📤 Extraer</button>
      </div>

      <div className="main-layout">
        <div className="config-panel">
          <fieldset>
            <legend>Compresión</legend>
            <input type="range" min="1" max="22" value={level} onChange={(e) => setLevel(e.target.value)} />
            <div className="level-info">
              <span>Nivel: <strong>{level}</strong></span>
              <span>Motor: {level >= 22 ? "7-Zip" : "Facebook"}</span>
              <hr />
              <span>Informe: {contexto.idInforme || 'N/A'}</span>
              <span>Perito: {contexto.usuario}</span>
            </div>
          </fieldset>

          {status === 'done' && contexto.idInforme && (
            <button className="hash-btn" onClick={handleHash}>🛡️ Registrar Hash en Informe</button>
          )}
        </div>

        <div className="status-window">
          <div className="info-grid">
            <div>Estado: <strong>{statusMessage}</strong></div>
            <div>Progreso: {progress}%</div>
          </div>
          <div className="progress-bar-container">
            <div className="progress-bar-fill" style={{ width: `${progress}%` }}></div>
          </div>
          <div className="file-log">
            <div className="log-line">{currentFile}</div>
            <div className="log-line truncate">{destPath}</div>
          </div>
        </div>
      </div>
    </div>
  );
}

export default App;
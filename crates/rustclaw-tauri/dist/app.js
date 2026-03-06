// RustClaw Tauri Frontend
const { invoke } = window.__TAURI__.core;

// DOM Elements
const setupScreen = document.getElementById('setup-screen');
const chatScreen = document.getElementById('chat-screen');
const setupForm = document.getElementById('setup-form');
const setupLoading = document.getElementById('setup-loading');
const setupError = document.getElementById('setup-error');
const providerSelect = document.getElementById('provider');
const apiKeyGroup = document.getElementById('api-key-group');
const apiKeyInput = document.getElementById('api-key');
const modelInput = document.getElementById('model');
const modelBadge = document.getElementById('model-badge');
const messagesContainer = document.getElementById('messages');
const messageInput = document.getElementById('message-input');
const btnSend = document.getElementById('btn-send');
const btnClear = document.getElementById('btn-clear');
const btnStats = document.getElementById('btn-stats');
const statsBar = document.getElementById('stats-bar');

// Default models per provider
const defaultModels = {
  anthropic: 'claude-sonnet-4-20250514',
  openai: 'gpt-4o',
  ollama: 'qwen3:8b',
};

let isProcessing = false;

// --- Init ---
async function init() {
  try {
    const config = await invoke('auto_load_config');
    if (config.configured) {
      showChat(config);
    } else {
      showSetup();
    }
  } catch (e) {
    showSetup();
  }
}

function showSetup() {
  setupLoading.classList.add('hidden');
  setupForm.classList.remove('hidden');
}

function showChat(config) {
  setupScreen.classList.add('hidden');
  chatScreen.classList.remove('hidden');
  modelBadge.textContent = `${config.provider} / ${config.model}`;
  messageInput.focus();
}

// --- Provider change ---
providerSelect.addEventListener('change', () => {
  const provider = providerSelect.value;
  modelInput.placeholder = defaultModels[provider] || '';

  if (provider === 'ollama') {
    apiKeyGroup.classList.add('hidden');
  } else {
    apiKeyGroup.classList.remove('hidden');
    apiKeyInput.placeholder = provider === 'anthropic' ? 'sk-ant-...' : 'sk-...';
  }
});

// --- Setup form submit ---
setupForm.addEventListener('submit', async (e) => {
  e.preventDefault();
  setupError.classList.add('hidden');

  const provider = providerSelect.value;
  const apiKey = apiKeyInput.value.trim();
  const model = modelInput.value.trim() || defaultModels[provider];

  if (provider !== 'ollama' && !apiKey) {
    setupError.textContent = 'API key is required.';
    setupError.classList.remove('hidden');
    return;
  }

  const btn = setupForm.querySelector('button');
  btn.disabled = true;
  btn.textContent = 'Connecting...';

  try {
    const config = await invoke('initialize_agent', {
      provider,
      apiKey: apiKey || '',
      model,
    });
    showChat(config);
  } catch (err) {
    setupError.textContent = `Error: ${err}`;
    setupError.classList.remove('hidden');
    btn.disabled = false;
    btn.textContent = 'Connect';
  }
});

// --- Send message ---
async function sendMessage() {
  const text = messageInput.value.trim();
  if (!text || isProcessing) return;

  isProcessing = true;
  btnSend.disabled = true;
  messageInput.value = '';
  autoResize();

  // Remove welcome message
  const welcome = messagesContainer.querySelector('.welcome-msg');
  if (welcome) welcome.remove();

  // Add user message
  addMessage('user', text);

  // Show typing indicator
  const typing = document.createElement('div');
  typing.className = 'typing-indicator';
  typing.textContent = 'Thinking...';
  messagesContainer.appendChild(typing);
  scrollToBottom();

  try {
    const result = await invoke('send_message', { message: text });
    typing.remove();

    if (result.success) {
      addMessage('assistant', result.message);
    } else {
      addMessage('error', result.error || 'Unknown error');
    }
  } catch (err) {
    typing.remove();
    addMessage('error', `Error: ${err}`);
  }

  isProcessing = false;
  btnSend.disabled = false;
  messageInput.focus();
}

function addMessage(role, content) {
  const div = document.createElement('div');
  div.className = `message ${role}`;

  const label = document.createElement('div');
  label.className = 'role-label';
  label.textContent = role === 'user' ? 'You' : role === 'error' ? 'Error' : 'RustClaw';
  div.appendChild(label);

  const body = document.createElement('div');
  body.textContent = content;
  div.appendChild(body);

  messagesContainer.appendChild(div);
  scrollToBottom();
}

function scrollToBottom() {
  messagesContainer.scrollTop = messagesContainer.scrollHeight;
}

// --- Input handling ---
btnSend.addEventListener('click', sendMessage);

messageInput.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault();
    sendMessage();
  }
});

messageInput.addEventListener('input', autoResize);

function autoResize() {
  messageInput.style.height = 'auto';
  messageInput.style.height = Math.min(messageInput.scrollHeight, 150) + 'px';
}

// --- Buttons ---
btnClear.addEventListener('click', async () => {
  try {
    await invoke('clear_conversation');
    messagesContainer.innerHTML = `
      <div class="welcome-msg">
        <p>Conversation cleared. Send a new message to start.</p>
      </div>`;
    statsBar.classList.add('hidden');
  } catch (err) {
    console.error('Clear failed:', err);
  }
});

btnStats.addEventListener('click', async () => {
  try {
    const [stats, cost] = await Promise.all([
      invoke('get_stats'),
      invoke('get_cost'),
    ]);
    statsBar.textContent = `${stats} | ${cost}`;
    statsBar.classList.toggle('hidden');
  } catch (err) {
    console.error('Stats failed:', err);
  }
});

// --- Start ---
init();

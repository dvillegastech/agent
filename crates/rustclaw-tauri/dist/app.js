// RustClaw Tauri Frontend - Phase 3/4: Streaming + Sessions
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

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
const btnSessions = document.getElementById('btn-sessions');
const btnCloseSidebar = document.getElementById('btn-close-sidebar');
const sessionsSidebar = document.getElementById('sessions-sidebar');
const sessionsList = document.getElementById('sessions-list');
const statsBar = document.getElementById('stats-bar');

// Default models per provider
const defaultModels = {
  anthropic: 'claude-sonnet-4-20250514',
  openai: 'gpt-4o',
  ollama: 'qwen3:8b',
};

let isProcessing = false;
let currentStreamingEl = null; // The assistant message element being streamed into
let currentStreamingBody = null; // The text body inside the streaming element
let activeToolCards = {}; // Track tool cards by ID

// --- Init ---
async function init() {
  setupEventListeners();
  await setupAgentEventListener();

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

// --- Agent Event Listener (streaming) ---
async function setupAgentEventListener() {
  await listen('agent-event', (event) => {
    const data = event.payload;
    if (!data || !data.type) return;

    switch (data.type) {
      case 'text_delta':
        handleTextDelta(data.text);
        break;
      case 'tool_start':
        handleToolStart(data.id, data.name, data.input);
        break;
      case 'tool_result':
        handleToolResult(data.id, data.name, data.output, data.is_error);
        break;
      case 'usage':
        handleUsage(data);
        break;
      case 'done':
        handleDone(data.text);
        break;
      case 'error':
        handleError(data.message);
        break;
    }
  });
}

function handleTextDelta(text) {
  if (!currentStreamingEl) {
    // Create the assistant message container
    currentStreamingEl = document.createElement('div');
    currentStreamingEl.className = 'message assistant';

    const label = document.createElement('div');
    label.className = 'role-label';
    label.textContent = 'RustClaw';
    currentStreamingEl.appendChild(label);

    currentStreamingBody = document.createElement('div');
    currentStreamingBody.className = 'message-body';
    currentStreamingEl.appendChild(currentStreamingBody);

    messagesContainer.appendChild(currentStreamingEl);
  }

  // Append streamed text
  currentStreamingBody.textContent += text;
  scrollToBottom();
}

function handleToolStart(id, name, input) {
  const card = document.createElement('div');
  card.className = 'tool-card running';
  card.innerHTML = `
    <div class="tool-header">
      <span class="tool-icon">&#9881;</span>
      <span class="tool-name">${escapeHtml(name)}</span>
      <span class="tool-status">Running...</span>
    </div>
    <div class="tool-input">${escapeHtml(input)}</div>
    <div class="tool-output"></div>
  `;

  // Insert after the current streaming element or at the end
  messagesContainer.appendChild(card);
  activeToolCards[id] = card;
  scrollToBottom();
}

function handleToolResult(id, name, output, isError) {
  const card = activeToolCards[id];
  if (card) {
    card.classList.remove('running');
    card.classList.add(isError ? 'error' : 'success');

    const statusEl = card.querySelector('.tool-status');
    statusEl.textContent = isError ? 'Error' : 'Done';

    const outputEl = card.querySelector('.tool-output');
    outputEl.textContent = output;
    outputEl.classList.add('visible');

    delete activeToolCards[id];
  }
  scrollToBottom();
}

function handleUsage(data) {
  statsBar.textContent = `Tokens: ${data.total_input} in / ${data.total_output} out | Cost: $${data.estimated_cost.toFixed(4)}`;
  statsBar.classList.remove('hidden');
}

function handleDone(_text) {
  // Reset streaming state
  currentStreamingEl = null;
  currentStreamingBody = null;
  activeToolCards = {};

  // Remove typing indicator if present
  const typing = messagesContainer.querySelector('.typing-indicator');
  if (typing) typing.remove();

  isProcessing = false;
  btnSend.disabled = false;
  messageInput.focus();
  scrollToBottom();
}

function handleError(message) {
  // Remove typing indicator
  const typing = messagesContainer.querySelector('.typing-indicator');
  if (typing) typing.remove();

  addMessage('error', message);

  currentStreamingEl = null;
  currentStreamingBody = null;
  activeToolCards = {};
  isProcessing = false;
  btnSend.disabled = false;
}

// --- Provider change ---
function setupEventListeners() {
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

  // Setup form submit
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

  // Send
  btnSend.addEventListener('click', sendMessage);

  messageInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  });

  messageInput.addEventListener('input', autoResize);

  // Clear
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

  // Stats
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

  // Sessions sidebar
  btnSessions.addEventListener('click', toggleSessions);
  btnCloseSidebar.addEventListener('click', () => {
    sessionsSidebar.classList.add('hidden');
  });
}

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

  // Reset streaming state
  currentStreamingEl = null;
  currentStreamingBody = null;
  activeToolCards = {};

  try {
    // The response comes back when complete, but streaming events
    // arrive via the agent-event listener above
    const result = await invoke('send_message', { message: text });

    // Remove typing indicator (may already be removed by handleDone)
    const t = messagesContainer.querySelector('.typing-indicator');
    if (t) t.remove();

    // If no streaming happened (e.g., error before any events), show the result
    if (!currentStreamingEl && result.success && result.message) {
      addMessage('assistant', result.message);
    } else if (!result.success) {
      addMessage('error', result.error || 'Unknown error');
    }
  } catch (err) {
    const t = messagesContainer.querySelector('.typing-indicator');
    if (t) t.remove();
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
  body.className = 'message-body';
  body.textContent = content;
  div.appendChild(body);

  messagesContainer.appendChild(div);
  scrollToBottom();
}

function scrollToBottom() {
  messagesContainer.scrollTop = messagesContainer.scrollHeight;
}

function autoResize() {
  messageInput.style.height = 'auto';
  messageInput.style.height = Math.min(messageInput.scrollHeight, 150) + 'px';
}

// --- Sessions ---
async function toggleSessions() {
  const isHidden = sessionsSidebar.classList.contains('hidden');
  if (isHidden) {
    sessionsSidebar.classList.remove('hidden');
    await loadSessionsList();
  } else {
    sessionsSidebar.classList.add('hidden');
  }
}

async function loadSessionsList() {
  try {
    const sessions = await invoke('list_sessions');
    if (sessions.length === 0) {
      sessionsList.innerHTML = '<p class="sidebar-empty">No saved sessions</p>';
      return;
    }

    sessionsList.innerHTML = '';
    for (const session of sessions) {
      const item = document.createElement('div');
      item.className = 'session-item';
      item.innerHTML = `
        <div class="session-info" data-id="${escapeHtml(session.id)}">
          <span class="session-date">${escapeHtml(session.updated_at)}</span>
          <span class="session-count">${session.message_count} messages</span>
        </div>
        <button class="session-delete" data-id="${escapeHtml(session.id)}" title="Delete">&#10005;</button>
      `;

      // Load session on click
      item.querySelector('.session-info').addEventListener('click', () => {
        loadSession(session.id);
      });

      // Delete session
      item.querySelector('.session-delete').addEventListener('click', (e) => {
        e.stopPropagation();
        deleteSession(session.id);
      });

      sessionsList.appendChild(item);
    }
  } catch (err) {
    sessionsList.innerHTML = `<p class="sidebar-empty">Error loading sessions</p>`;
    console.error('Failed to load sessions:', err);
  }
}

async function loadSession(sessionId) {
  try {
    const messages = await invoke('load_session', { sessionId });
    messagesContainer.innerHTML = '';

    for (const msg of messages) {
      if (msg.role === 'user') {
        // Extract text content
        const text = extractTextFromContent(msg.content);
        if (text) addMessage('user', text);
      } else if (msg.role === 'assistant') {
        const text = extractTextFromContent(msg.content);
        if (text) addMessage('assistant', text);
      }
    }

    sessionsSidebar.classList.add('hidden');
    scrollToBottom();
  } catch (err) {
    console.error('Failed to load session:', err);
  }
}

function extractTextFromContent(content) {
  if (typeof content === 'string') return content;
  if (Array.isArray(content)) {
    return content
      .filter((b) => b.type === 'text')
      .map((b) => b.text)
      .join('');
  }
  if (content && content.Blocks) {
    return content.Blocks
      .filter((b) => b.Text)
      .map((b) => b.Text.text || b.Text)
      .join('');
  }
  return '';
}

async function deleteSession(sessionId) {
  try {
    await invoke('delete_session', { sessionId });
    await loadSessionsList();
  } catch (err) {
    console.error('Failed to delete session:', err);
  }
}

// --- Utilities ---
function escapeHtml(text) {
  const div = document.createElement('div');
  div.textContent = text;
  return div.innerHTML;
}

// --- Start ---
init();

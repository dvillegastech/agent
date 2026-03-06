use crate::types::{Message, MessageContent, Role};

/// Gestiona el historial de conversación con ventana deslizante.
pub struct Conversation {
    messages: Vec<Message>,
    max_turns: usize,
}

impl Conversation {
    pub fn new(max_turns: usize) -> Self {
        Self {
            messages: Vec::new(),
            max_turns,
        }
    }

    /// Agrega un mensaje del usuario.
    pub fn add_user_message(&mut self, text: &str) {
        self.messages.push(Message {
            role: Role::User,
            content: MessageContent::Text(text.to_string()),
        });
        self.trim();
    }

    /// Agrega un mensaje del asistente.
    pub fn add_assistant_message(&mut self, content: MessageContent) {
        self.messages.push(Message {
            role: Role::Assistant,
            content,
        });
        self.trim();
    }

    /// Agrega un mensaje con resultados de herramientas.
    pub fn add_tool_results(&mut self, content: MessageContent) {
        self.messages.push(Message {
            role: Role::User,
            content,
        });
        self.trim();
    }

    /// Retorna todos los mensajes.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Limpia la conversación.
    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// Número de mensajes en la conversación.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Mantiene la conversación dentro del límite de turnos.
    /// Preserva siempre el primer mensaje del usuario para contexto.
    fn trim(&mut self) {
        let max_messages = self.max_turns * 2; // Cada turno = user + assistant
        if self.messages.len() > max_messages && self.messages.len() > 2 {
            let excess = self.messages.len() - max_messages;
            // Mantener primer mensaje, eliminar los siguientes más antiguos
            self.messages.drain(1..=excess);
        }
    }
}

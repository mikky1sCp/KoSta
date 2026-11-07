import { Loader2, MessageCircle } from 'lucide-react';
import type { ChatDetails, ChatItem, UiMessage, WsStatus } from '../types';
import { ChatHeader } from './ChatHeader';
import { Composer } from './Composer';
import { MessageList } from './MessageList';
import { cn } from '../utils/classNames';

interface ChatPanelProps {
  chat: ChatItem | null;
  details: ChatDetails | null;
  messages: UiMessage[];
  historyLoading: boolean;
  loadingMore: boolean;
  hasMore: boolean;
  typingLabel: string | null;
  wsStatus: WsStatus;
  onBack: () => void;
  onOpenInfo: () => void;
  onSend: (text: string) => boolean;
  onLoadMore: () => Promise<boolean>;
  onTypingInput: (hasText: boolean) => void;
  className?: string;
}

function EmptyChat() {
  return (
    <div className="flex h-full flex-1 flex-col items-center justify-center px-6 text-center">
      <span className="logo-glow mb-4 flex h-16 w-16 items-center justify-center rounded-3xl bg-gradient-to-br from-indigo-500 to-violet-600 text-white">
        <MessageCircle className="h-8 w-8" strokeWidth={2.2} aria-hidden="true" />
      </span>
      <h2 className="text-lg font-semibold text-ink">Добро пожаловать в KoSta</h2>
      <p className="mt-2 max-w-sm text-sm text-muted">
        Выберите чат слева или начните новый диалог. История сообщений и
        непрочитанные подтянутся автоматически.
      </p>
    </div>
  );
}

export function ChatPanel({
  chat,
  details,
  messages,
  historyLoading,
  loadingMore,
  hasMore,
  typingLabel,
  wsStatus,
  onBack,
  onOpenInfo,
  onSend,
  onLoadMore,
  onTypingInput,
  className,
}: ChatPanelProps) {
  return (
    <main
      className={cn(
        'h-full min-h-0 flex-1 flex-col bg-panel/30 backdrop-blur-sm',
        className,
      )}
    >
      {!chat ? (
        <EmptyChat />
      ) : (
        <>
          <ChatHeader
            chat={chat}
            details={details}
            typingLabel={typingLabel}
            onBack={onBack}
            onOpenInfo={onOpenInfo}
          />

          {wsStatus !== 'connected' && (
            <div className="flex items-center justify-center gap-2 border-b border-line bg-warn/5 px-4 py-1.5 text-[11px] text-warn">
              <Loader2 className="h-3 w-3 animate-spin" aria-hidden="true" />
              Восстанавливаем соединение…
            </div>
          )}

          <MessageList
            chatId={chat.id}
            messages={messages}
            loading={historyLoading}
            loadingMore={loadingMore}
            hasMore={hasMore}
            typingLabel={typingLabel}
            isGroup={chat.is_group}
            participants={details?.participants ?? []}
            onLoadMore={onLoadMore}
          />

          <Composer
            key={chat.id}
            chatId={chat.id}
            connected={wsStatus === 'connected'}
            onSend={onSend}
            onTypingInput={onTypingInput}
          />
        </>
      )}
    </main>
  );
}

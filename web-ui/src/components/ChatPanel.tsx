import { Loader2, MessageCircle, Search } from 'lucide-react';
import type { Api } from '../api';
import type { ChatDetails, ChatItem, UiMessage, WsStatus } from '../types';
import { ChatHeader } from './ChatHeader';
import { Composer } from './Composer';
import { MessageList } from './MessageList';
import { cn } from '../utils/classNames';

interface ChatPanelProps {
  api: Api;
  chat: ChatItem | null;
  details: ChatDetails | null;
  messages: UiMessage[];
  historyLoading: boolean;
  loadingMore: boolean;
  hasMore: boolean;
  typingLabel: string | null;
  wsStatus: WsStatus;
  /** id первого сообщения непрочитанного блока — над ним рисуем разделитель. */
  unreadDividerMessageId: number | null;
  onBack: () => void;
  onOpenInfo: () => void;
  onSend: (text: string) => boolean;
  onUpload: (
    file: File,
    caption: string,
    onProgress: (percent: number) => void,
  ) => Promise<boolean>;
  onLoadMore: () => Promise<boolean>;
  onTypingInput: (hasText: boolean) => void;
  onOpenImage: (message: UiMessage, url: string) => void;
  onDownload: (message: UiMessage) => Promise<void>;
  onCopyMessage: (message: UiMessage) => void;
  initialDraft: string;
  onDraftChange: (chatId: number, text: string) => void;
  className?: string;
}

function EmptyChat() {
  return (
    <div className="flex h-full flex-1 flex-col items-center justify-center px-6 text-center">
      <div className="relative mb-5">
        <span className="flex h-14 w-14 items-center justify-center rounded-2xl bg-accent text-white">
          <MessageCircle className="h-7 w-7" strokeWidth={2.2} aria-hidden="true" />
        </span>
        <span className="absolute -bottom-1 -right-1 flex h-7 w-7 items-center justify-center rounded-full border border-line bg-panel text-accent-strong">
          <Search className="h-3.5 w-3.5" aria-hidden="true" />
        </span>
      </div>

      <h2 className="text-lg font-semibold tracking-tight text-ink">
        Добро пожаловать в KoSta
      </h2>
      <p className="mt-2 max-w-sm text-sm leading-relaxed text-muted">
        Выберите чат слева или начните новый диалог. История и непрочитанные
        подтянутся автоматически.
      </p>
      <p className="mt-4 inline-flex items-center gap-2 rounded-full border border-line bg-panel px-3 py-1.5 text-[11px] text-muted">
        <kbd className="rounded-md border border-line bg-panel-soft px-1.5 py-0.5 font-sans text-[10px] font-medium text-ink">
          Ctrl+K
        </kbd>
        поиск по чатам
      </p>
    </div>
  );
}

export function ChatPanel({
  api,
  chat,
  details,
  messages,
  historyLoading,
  loadingMore,
  hasMore,
  typingLabel,
  wsStatus,
  unreadDividerMessageId,
  onBack,
  onOpenInfo,
  onSend,
  onUpload,
  onLoadMore,
  onTypingInput,
  onOpenImage,
  onDownload,
  onCopyMessage,
  initialDraft,
  onDraftChange,
  className,
}: ChatPanelProps) {
  return (
    <main
      className={cn(
        'h-full min-h-0 flex-1 flex-col bg-app',
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
            api={api}
            unreadDividerMessageId={unreadDividerMessageId}
            onLoadMore={onLoadMore}
            onOpenImage={onOpenImage}
            onDownload={onDownload}
            onCopyMessage={onCopyMessage}
          />

          <Composer
            key={chat.id}
            chatId={chat.id}
            connected={wsStatus === 'connected'}
            initialDraft={initialDraft}
            onDraftChange={onDraftChange}
            onSend={onSend}
            onUpload={onUpload}
            onTypingInput={onTypingInput}
          />
        </>
      )}
    </main>
  );
}

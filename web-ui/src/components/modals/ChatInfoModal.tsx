import { useEffect, useState } from 'react';
import {
  AlertCircle,
  Crown,
  LogOut,
  Pencil,
  Plus,
  Trash2,
  UsersRound,
} from 'lucide-react';
import type { Api } from '../../api';
import type { ChatDetails, Participant } from '../../types';
import type { ToastApi } from '../../hooks/useToasts';
import { getErrorMessage } from '../../utils/errors';
import { pluralize } from '../../utils/format';
import { Modal } from '../ui/Modal';
import { Avatar } from '../ui/Avatar';
import { Button } from '../ui/Button';
import { Spinner } from '../ui/Spinner';
import { TextInput } from '../ui/TextInput';

interface ChatInfoModalProps {
  api: Api;
  toast: ToastApi;
  chatId: number;
  myUserId: number;
  initialDetails: ChatDetails | null;
  onClose: () => void;
  onDetailsChange: (details: ChatDetails) => void;
  onChatsChanged: () => void;
  onLeftChat: (chatId: number) => void;
}

export function ChatInfoModal({
  api,
  toast,
  chatId,
  myUserId,
  initialDetails,
  onClose,
  onDetailsChange,
  onChatsChanged,
  onLeftChat,
}: ChatInfoModalProps) {
  const [details, setDetails] = useState<ChatDetails | null>(initialDetails);
  const [loading, setLoading] = useState(initialDetails === null);
  const [loadError, setLoadError] = useState('');

  const [addPhone, setAddPhone] = useState('');
  const [adding, setAdding] = useState(false);
  const [removingUserId, setRemovingUserId] = useState<number | null>(null);
  const [leaving, setLeaving] = useState(false);

  const [titleDraft, setTitleDraft] = useState(initialDetails?.chat.title ?? '');
  const [savingTitle, setSavingTitle] = useState(false);

  function applyDetails(next: ChatDetails) {
    setDetails(next);
    setTitleDraft(next.chat.title);
    onDetailsChange(next);
  }

  async function reloadDetails(): Promise<ChatDetails> {
    const next = await api.chatDetails(chatId);
    applyDetails(next);
    return next;
  }

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setLoadError('');
    api
      .chatDetails(chatId)
      .then((next) => {
        if (!cancelled) {
          applyDetails(next);
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setLoadError(getErrorMessage(error));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
    // applyDetails/onDetailsChange — стабильные колбэки из App.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, chatId]);

  const isGroup = details?.chat.is_group ?? false;
  const isAdmin = details?.is_admin ?? false;
  const participants = details?.participants ?? [];
  const title = details?.chat.title ?? 'Чат';

  async function handleAddParticipant() {
    const phone = addPhone.trim();
    if (!phone) {
      toast.error('Введите номер телефона');
      return;
    }
    setAdding(true);
    try {
      const next = await api.addParticipant(chatId, phone);
      applyDetails(next);
      setAddPhone('');
      onChatsChanged();
      toast.success('Участник добавлен');
    } catch (error) {
      toast.error(getErrorMessage(error));
    } finally {
      setAdding(false);
    }
  }

  async function handleRename() {
    const nextTitle = titleDraft.trim();
    if (!nextTitle) {
      toast.error('Название не может быть пустым');
      return;
    }
    if (nextTitle === details?.chat.title) {
      return;
    }
    setSavingTitle(true);
    try {
      const next = await api.renameChat(chatId, nextTitle);
      applyDetails(next);
      onChatsChanged();
      toast.success('Название обновлено');
    } catch (error) {
      toast.error(getErrorMessage(error));
    } finally {
      setSavingTitle(false);
    }
  }

  async function handleRemoveParticipant(participant: Participant) {
    if (!window.confirm(`Удалить ${participant.phone} из группы?`)) {
      return;
    }
    setRemovingUserId(participant.user_id);
    try {
      await api.removeParticipant(chatId, participant.user_id);
      await reloadDetails();
      onChatsChanged();
      toast.success('Участник удалён');
    } catch (error) {
      toast.error(getErrorMessage(error));
    } finally {
      setRemovingUserId(null);
    }
  }

  async function handleLeave() {
    if (!window.confirm('Выйти из группы? Вы больше не будете получать сообщения.')) {
      return;
    }
    setLeaving(true);
    try {
      const result = await api.removeParticipant(chatId, myUserId);
      onLeftChat(chatId);
      toast.success(result.deleted ? 'Группа удалена' : 'Вы вышли из группы');
    } catch (error) {
      toast.error(getErrorMessage(error));
    } finally {
      setLeaving(false);
    }
  }

  return (
    <Modal
      title="Информация о чате"
      subtitle={isGroup ? 'Групповой чат' : 'Личный чат'}
      icon={<UsersRound className="h-4 w-4" aria-hidden="true" />}
      onClose={onClose}
      size="md"
      footer={
        isGroup ? (
          <Button
            variant="danger"
            fullWidth
            icon={<LogOut className="h-4 w-4" aria-hidden="true" />}
            loading={leaving}
            onClick={() => void handleLeave()}
          >
            Выйти из группы
          </Button>
        ) : undefined
      }
    >
      {loading && !details ? (
        <div className="flex items-center justify-center gap-2 py-12 text-sm text-muted">
          <Spinner className="h-4 w-4" /> Загружаем информацию…
        </div>
      ) : loadError && !details ? (
        <div
          role="alert"
          className="flex items-start gap-2 rounded-xl border border-danger/25 bg-danger/10 px-3 py-2.5 text-sm text-danger"
        >
          <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
          <span className="break-words">{loadError}</span>
        </div>
      ) : details ? (
        <div className="space-y-5">
          <div className="flex flex-col items-center text-center">
            <Avatar
              title={title}
              isGroup={isGroup}
              online={!isGroup && details.chat.is_online === true}
              size="xl"
            />
            <h3 className="mt-3 truncate text-lg font-semibold text-ink">{title}</h3>
            <p className="mt-0.5 text-xs text-muted">
              {isGroup
                ? `${participants.length} ${pluralize(
                    participants.length,
                    'участник',
                    'участника',
                    'участников',
                  )}`
                : details.chat.is_online
                  ? 'онлайн'
                  : 'не в сети'}
            </p>
            {details.is_admin && isGroup && (
              <span className="mt-2 inline-flex items-center gap-1 rounded-full bg-accent-soft px-2.5 py-1 text-[11px] font-medium text-accent-strong">
                <Crown className="h-3 w-3" aria-hidden="true" />
                Вы администратор
              </span>
            )}
          </div>

          {loadError && (
            <div
              role="alert"
              className="flex items-start gap-2 rounded-xl border border-danger/25 bg-danger/10 px-3 py-2.5 text-sm text-danger"
            >
              <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
              <span className="break-words">{loadError}</span>
            </div>
          )}

          {isGroup && isAdmin && (
            <section>
              <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted">
                Название группы
              </h4>
              <div className="flex items-end gap-2">
                <TextInput
                  containerClassName="flex-1"
                  aria-label="Название группы"
                  value={titleDraft}
                  onChange={(event) => setTitleDraft(event.target.value)}
                  maxLength={128}
                  disabled={savingTitle}
                />
                <Button
                  variant="secondary"
                  size="md"
                  loading={savingTitle}
                  disabled={titleDraft.trim() === details.chat.title}
                  onClick={() => void handleRename()}
                  icon={<Pencil className="h-4 w-4" aria-hidden="true" />}
                >
                  Сохранить
                </Button>
              </div>
            </section>
          )}

          <section>
            <h4 className="mb-2 flex items-center justify-between text-xs font-semibold uppercase tracking-wide text-muted">
              <span>Участники</span>
              <span className="normal-case tracking-normal">{participants.length}</span>
            </h4>
            <ul className="divide-y divide-line-soft overflow-hidden rounded-xl border border-line">
              {participants.map((participant) => {
                const isMe = participant.user_id === myUserId;
                return (
                  <li
                    key={participant.user_id}
                    className="flex items-center gap-3 bg-panel-soft px-3 py-2.5"
                  >
                    <Avatar
                      title={participant.phone}
                      isGroup={false}
                      online={participant.is_online}
                      size="sm"
                    />
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm font-medium text-ink">
                        {participant.phone}
                      </p>
                      <p className="text-[11px] text-muted">
                        {participant.is_online ? 'онлайн' : 'не в сети'}
                      </p>
                    </div>
                    <div className="flex shrink-0 items-center gap-1.5">
                      {isMe && (
                        <span className="rounded-full bg-accent-soft px-2 py-0.5 text-[10px] font-medium text-accent-strong">
                          Вы
                        </span>
                      )}
                      {participant.is_admin && (
                        <span
                          className="rounded-full bg-warn/10 px-2 py-0.5 text-[10px] font-medium text-warn"
                          title="Администратор"
                        >
                          админ
                        </span>
                      )}
                      {isGroup && isAdmin && !isMe && (
                        <button
                          type="button"
                          onClick={() => void handleRemoveParticipant(participant)}
                          disabled={removingUserId !== null}
                          aria-label={`Удалить ${participant.phone}`}
                          title="Удалить участника"
                          className="flex h-7 w-7 items-center justify-center rounded-lg text-muted transition-colors hover:bg-danger/10 hover:text-danger disabled:opacity-50"
                        >
                          {removingUserId === participant.user_id ? (
                            <Spinner className="h-3.5 w-3.5" />
                          ) : (
                            <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
                          )}
                        </button>
                      )}
                    </div>
                  </li>
                );
              })}
            </ul>
          </section>

          {isGroup && isAdmin && (
            <section>
              <h4 className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted">
                Добавить участника
              </h4>
              <div className="flex items-end gap-2">
                <TextInput
                  containerClassName="flex-1"
                  aria-label="Телефон нового участника"
                  type="tel"
                  inputMode="tel"
                  placeholder="+79990000001"
                  value={addPhone}
                  onChange={(event) => setAddPhone(event.target.value)}
                  disabled={adding}
                />
                <Button
                  size="md"
                  loading={adding}
                  onClick={() => void handleAddParticipant()}
                  icon={<Plus className="h-4 w-4" aria-hidden="true" />}
                >
                  Добавить
                </Button>
              </div>
            </section>
          )}
        </div>
      ) : null}
    </Modal>
  );
}

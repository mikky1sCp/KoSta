import { useState, type FormEvent } from 'react';
import { AlertCircle, Plus, Trash2, UsersRound, X } from 'lucide-react';
import type { Api } from '../../api';
import type { ChatDetails } from '../../types';
import type { ToastApi } from '../../hooks/useToasts';
import { getErrorMessage } from '../../utils/errors';
import { Modal } from '../ui/Modal';
import { TextInput } from '../ui/TextInput';
import { Button } from '../ui/Button';

const PHONE_PATTERN = /^\+?\d{5,20}$/;
const MAX_PHONES = 199;

interface NewGroupModalProps {
  api: Api;
  toast: ToastApi;
  onClose: () => void;
  onCreated: (details: ChatDetails) => void;
}

export function NewGroupModal({ api, toast, onClose, onCreated }: NewGroupModalProps) {
  const [title, setTitle] = useState('');
  const [phones, setPhones] = useState<string[]>(['']);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  function updatePhone(index: number, value: string) {
    setPhones((current) =>
      current.map((phone, phoneIndex) => (phoneIndex === index ? value : phone)),
    );
  }

  function addPhoneRow() {
    setPhones((current) => (current.length >= MAX_PHONES ? current : [...current, '']));
  }

  function removePhoneRow(index: number) {
    setPhones((current) => current.filter((_, phoneIndex) => phoneIndex !== index));
  }

  async function handleSubmit(event?: FormEvent<HTMLFormElement>) {
    event?.preventDefault();
    if (loading) {
      return;
    }
    const normalizedTitle = title.trim();
    if (normalizedTitle.length === 0) {
      setError('Введите название группы');
      return;
    }
    if (normalizedTitle.length > 128) {
      setError('Название должно быть не длиннее 128 символов');
      return;
    }

    const normalizedPhones: string[] = [];
    for (const rawPhone of phones) {
      const phone = rawPhone.trim();
      if (!phone) {
        continue;
      }
      if (!PHONE_PATTERN.test(phone)) {
        setError(`Некорректный номер: ${phone}`);
        return;
      }
      if (!normalizedPhones.includes(phone)) {
        normalizedPhones.push(phone);
      }
    }

    setError('');
    setLoading(true);
    try {
      const details = await api.createGroup(normalizedTitle, normalizedPhones);
      onCreated(details);
    } catch (submitError) {
      const message = getErrorMessage(submitError);
      setError(message);
      toast.error(message);
    } finally {
      setLoading(false);
    }
  }

  return (
    <Modal
      title="Новая группа"
      subtitle="Соберите участников по номерам телефонов"
      icon={<UsersRound className="h-4 w-4" aria-hidden="true" />}
      onClose={onClose}
      footer={
        <div className="flex gap-2">
          <Button variant="secondary" fullWidth onClick={onClose} disabled={loading}>
            Отмена
          </Button>
          <Button type="submit" form="new-group-form" fullWidth loading={loading}>
            Создать группу
          </Button>
        </div>
      }
    >
      <form id="new-group-form" onSubmit={handleSubmit} className="space-y-4" noValidate>
        <TextInput
          label="Название группы"
          autoFocus
          placeholder="Например, «Друзья»"
          value={title}
          onChange={(event) => setTitle(event.target.value)}
          disabled={loading}
          maxLength={128}
        />

        <div>
          <div className="mb-1.5 flex items-center justify-between">
            <span className="text-xs font-medium text-muted">
              Участники {phones.filter((phone) => phone.trim()).length > 0
                ? `(${phones.filter((phone) => phone.trim()).length})`
                : ''}
            </span>
            <button
              type="button"
              onClick={addPhoneRow}
              disabled={loading || phones.length >= MAX_PHONES}
              className="flex items-center gap-1 rounded-lg px-2 py-1 text-xs font-medium text-accent-strong transition-colors hover:bg-accent-soft disabled:opacity-50"
            >
              <Plus className="h-3.5 w-3.5" aria-hidden="true" />
              Добавить
            </button>
          </div>

          {phones.length === 0 ? (
            <p className="rounded-xl border border-dashed border-line px-3 py-4 text-center text-xs text-muted">
              Участников можно добавить позже
            </p>
          ) : (
            <div className="space-y-2">
              {phones.map((phone, index) => (
                <div key={index} className="flex items-center gap-2">
                  <TextInput
                    containerClassName="flex-1"
                    aria-label={`Телефон участника ${index + 1}`}
                    type="tel"
                    inputMode="tel"
                    placeholder="+79990000001"
                    value={phone}
                    onChange={(event) => updatePhone(index, event.target.value)}
                    disabled={loading}
                  />
                  <button
                    type="button"
                    onClick={() => removePhoneRow(index)}
                    disabled={loading}
                    aria-label="Убрать участника"
                    className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl border border-line bg-panel-soft text-muted transition-colors hover:border-danger/30 hover:bg-danger/10 hover:text-danger disabled:opacity-50"
                  >
                    {index === 0 && phones.length === 1 ? (
                      <X className="h-4 w-4" aria-hidden="true" />
                    ) : (
                      <Trash2 className="h-4 w-4" aria-hidden="true" />
                    )}
                  </button>
                </div>
              ))}
            </div>
          )}
          <p className="mt-2 text-[11px] text-muted">
            Вы станете администратором группы. Участников можно добавить позже в
            информации о чате.
          </p>
        </div>

        {error && (
          <div
            role="alert"
            className="flex items-start gap-2 rounded-xl border border-danger/25 bg-danger/10 px-3 py-2.5 text-sm text-danger"
          >
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
            <span className="break-words">{error}</span>
          </div>
        )}
      </form>
    </Modal>
  );
}

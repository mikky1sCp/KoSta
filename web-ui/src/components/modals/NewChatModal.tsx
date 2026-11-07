import { useState, type FormEvent } from 'react';
import { AlertCircle, MessageSquarePlus, Phone } from 'lucide-react';
import type { Api } from '../../api';
import type { ChatDetails } from '../../types';
import type { ToastApi } from '../../hooks/useToasts';
import { getErrorMessage } from '../../utils/errors';
import { Modal } from '../ui/Modal';
import { TextInput } from '../ui/TextInput';
import { Button } from '../ui/Button';

const PHONE_PATTERN = /^\+?\d{5,20}$/;

interface NewChatModalProps {
  api: Api;
  toast: ToastApi;
  onClose: () => void;
  onCreated: (details: ChatDetails) => void;
}

export function NewChatModal({ api, toast, onClose, onCreated }: NewChatModalProps) {
  const [phone, setPhone] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  async function handleSubmit(event?: FormEvent<HTMLFormElement>) {
    event?.preventDefault();
    if (loading) {
      return;
    }
    const normalized = phone.trim();
    if (!PHONE_PATTERN.test(normalized)) {
      setError('Введите телефон: необязательный «+» и от 5 до 20 цифр');
      return;
    }
    setError('');
    setLoading(true);
    try {
      const details = await api.createPrivate(normalized);
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
      title="Новый чат"
      subtitle="Личная переписка по номеру телефона"
      icon={<MessageSquarePlus className="h-4 w-4" aria-hidden="true" />}
      onClose={onClose}
      size="sm"
      footer={
        <div className="flex gap-2">
          <Button variant="secondary" fullWidth onClick={onClose} disabled={loading}>
            Отмена
          </Button>
          <Button
            type="submit"
            form="new-chat-form"
            fullWidth
            loading={loading}
          >
            Начать чат
          </Button>
        </div>
      }
    >
      <form id="new-chat-form" onSubmit={handleSubmit} className="space-y-4" noValidate>
        <TextInput
          label="Телефон собеседника"
          icon={<Phone className="h-4 w-4" aria-hidden="true" />}
          type="tel"
          inputMode="tel"
          autoFocus
          placeholder="+79990000001"
          value={phone}
          onChange={(event) => setPhone(event.target.value)}
          disabled={loading}
          hint="Если диалога ещё нет, он будет создан автоматически"
        />
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

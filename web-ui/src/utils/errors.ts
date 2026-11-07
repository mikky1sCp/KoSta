import { ApiError } from '../api';

/** Человекочитаемый текст ошибки для тоста/инлайн-сообщения. */
export function getErrorMessage(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.status === 0) {
      return 'Не удалось подключиться к серверу. Проверьте соединение.';
    }
    return error.message;
  }
  if (error instanceof Error && error.message) {
    return error.message;
  }
  return 'Что-то пошло не так. Попробуйте ещё раз.';
}

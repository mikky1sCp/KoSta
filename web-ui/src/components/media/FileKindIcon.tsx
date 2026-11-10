import { File, FileArchive, FileImage, FileText, Film, Music } from 'lucide-react';
import { mediaKind, type MediaKind } from '../../utils/media';
import { cn } from '../../utils/classNames';

const ICONS: Record<MediaKind, typeof File> = {
  image: FileImage,
  video: Film,
  audio: Music,
  archive: FileArchive,
  document: FileText,
  file: File,
};

interface FileKindIconProps {
  mime: string | null | undefined;
  className?: string;
}

/** Иконка по MIME-типу вложения. Декоративная — подпись есть рядом в тексте. */
export function FileKindIcon({ mime, className }: FileKindIconProps) {
  const Icon = ICONS[mediaKind(mime)];
  return <Icon className={cn('h-4 w-4', className)} aria-hidden="true" />;
}

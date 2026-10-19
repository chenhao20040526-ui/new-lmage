import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';
import { motion, AnimatePresence } from 'framer-motion';
import { toast } from 'sonner';
import {
  Calendar,
  Check,
  Globe,
  ImagePlus,
  Images,
  KeyRound,
  SquareCheck,
  Trash2,
  Upload,
  X,
  ExternalLink,
} from 'lucide-react';
import { api } from '../../services/api';
import type { Album, Image } from '../../services/api';
import { Loader } from '../../components/common/Loader';
import { EmptyState } from '../../components/common/EmptyState';
import { ConfirmSheet } from '../../components/common/ConfirmSheet';
import { ImagePickerModal } from '../../components/common/ImagePickerModal';
import { ImageLightbox } from '../../components/common/ImageLightbox';
import { PageHeader } from '../../components/common/PageHeader';
import { FadeInUp } from '../../components/common/FadeInUp';
import { formatBytes } from '../../components/common/format';
import { processImageBeforeUpload } from '../../utils/imageProcessor';
import '../../components/common/pageActions.css';
import './albumdetail.css';

type ConfirmState = { kind: 'album' } | { kind: 'image'; image: Image } | null;

const isPasswordError = (err: unknown): boolean =>
  typeof err === 'object' &&
  err !== null &&
  'message' in err &&
  typeof (err as { message?: unknown }).message === 'string' &&
  /密码|提取码/.test((err as { message: string }).message);

export const AlbumDetail: React.FC = () => {
  const { albumid } = useParams<{ albumid: string }>();
  const navigate = useNavigate();

  const [album, setAlbum] = useState<Album | null>(null);
  const [images, setImages] = useState<Image[]>([]);
  const [loading, setLoading] = useState(true);
  const [notFound, setNotFound] = useState(false);

  // 提取码解锁
  const [locked, setLocked] = useState(false);
  const [password, setPassword] = useState('');
  const [unlocking, setUnlocking] = useState(false);
  const [accessPwd, setAccessPwd] = useState('');
  const unlockRef = useRef<HTMLInputElement>(null);

  // 光箱 / 多选 / 批量移出
  const [lightbox, setLightbox] = useState<Image | null>(null);
  const [selecting, setSelecting] = useState(false);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [removing, setRemoving] = useState(false);

  // 选图弹层 / 直接上传
  const [showPicker, setShowPicker] = useState(false);
  const [adding, setAdding] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [uploadStatus, setUploadStatus] = useState('');
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Telegra.ph 发布与即时预览弹层
  const [showTgModal, setShowTgModal] = useState(false);
  const [tgPublishing, setTgPublishing] = useState(false);
  const [tgResultUrl, setTgResultUrl] = useState('');

  // 删除确认
  const [confirm, setConfirm] = useState<ConfirmState>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(
    async (pwd?: string) => {
      if (!albumid) return;
      setLoading(true);
      setNotFound(false);
      try {
        const res = await api.getAlbumDetail(albumid, pwd);
        setAlbum(res.album);
        setImages(res.images);
        setLocked(false);
        setPassword('');
        if (pwd) setAccessPwd(pwd);
      } catch (err) {
        if (isPasswordError(err)) {
          setLocked(true);
          setAlbum(null);
          setImages([]);
          if (pwd) toast.error('提取码错误，请重新输入');
        } else {
          setNotFound(true);
          toast.error((err as Error)?.message ?? '加载相册失败');
        }
      } finally {
        setLoading(false);
      }
    },
    [albumid]
  );

  useEffect(() => {
    load();
  }, [albumid, load]);

  useEffect(() => {
    if (locked) unlockRef.current?.focus();
  }, [locked]);

  const handleUnlock = async (e: React.FormEvent) => {
    e.preventDefault();
    if (unlocking) return;
    if (!password.trim()) {
      toast.warning('请输入相册提取码');
      return;
    }
    setUnlocking(true);
    try {
      await load(password);
    } finally {
      setUnlocking(false);
    }
  };

  const exitSelectMode = () => {
    setSelecting(false);
    setSelectedIds([]);
  };

  const handleItemClick = (img: Image) => {
    if (selecting) {
      setSelectedIds((prev) =>
        prev.includes(img.id) ? prev.filter((x) => x !== img.id) : [...prev, img.id]
      );
    } else {
      setLightbox(img);
    }
  };

  const handleRemoveSelected = async () => {
    if (!albumid || selectedIds.length === 0) return;
    setRemoving(true);
    try {
      await api.modifyAlbumImages(albumid, selectedIds, 'remove');
      toast.success(`已从相册移出 ${selectedIds.length} 张图片`);
      const removed = new Set(selectedIds);
      setImages((prev) => prev.filter((img) => !removed.has(img.id)));
      exitSelectMode();
    } catch (err) {
      toast.error((err as Error)?.message ?? '移出图片失败');
    } finally {
      setRemoving(false);
    }
  };

  // 1. 相册内直接上传图片（集成自动 WebP->JPG 及 >3MB 自动压到 1280p）
  const handleDirectUploadFiles = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const rawFiles = e.target.files;
    if (!rawFiles || rawFiles.length === 0 || !albumid) return;

    const files = Array.from(rawFiles).filter((f) => f.type.startsWith('image/'));
    if (files.length === 0) {
      toast.warning('请选择有效的图片文件');
      return;
    }

    setUploading(true);
    let successCount = 0;
    const newImageIds: string[] = [];

    for (let i = 0; i < files.length; i++) {
      setUploadStatus(`⚡ 正在处理与上传 (${i + 1}/${files.length}): ${files[i].name}...`);
      try {
        // 前端自动压缩与转码
        const processedFile = await processImageBeforeUpload(files[i]);
        
        // 调用上传接口
        const formData = new FormData();
        formData.append('file', processedFile);
        const res = await api.uploadImage(formData); // 假设 api.uploadImage 存在
        
        const imgId = res.id || res.data?.id;
        if (imgId) {
          newImageIds.push(imgId);
          successCount++;
        }
      } catch (err) {
        console.error(`上传 ${files[i].name} 失败:`, err);
      }
    }

    if (newImageIds.length > 0) {
      try {
        await api.modifyAlbumImages(albumid, newImageIds, 'add');
        toast.success(`成功上传并归档 ${successCount} 张图片！`);
        await load(accessPwd || undefined);
      } catch (err) {
        toast.error('归档到相册失败');
      }
    }

    setUploading(false);
    setUploadStatus('');
    if (fileInputRef.current) fileInputRef.current.value = '';
  };

  const handleAddImages = async (ids: string[]) => {
    if (!albumid || ids.length === 0) return;
    setAdding(true);
    try {
      await api.modifyAlbumImages(albumid, ids, 'add');
      toast.success(`已加入 ${ids.length} 张图片到「${album?.name ?? '本相册'}」`);
      setShowPicker(false);
      await load(accessPwd || undefined);
    } catch (err) {
      toast.error((err as Error)?.message ?? '加入图片失败');
    } finally {
      setAdding(false);
    }
  };

  // 2. 一键发布到 Telegra.ph 并生成即时预览链接
  const handlePublishToTelegraph = async () => {
    if (!album || images.length === 0) {
      toast.warning('相册内暂无图片，无法生成 Telegra.ph 页面');
      return;
    }

    setTgPublishing(true);
    try {
      const contentNodes = images.map((img) => ({
        tag: 'figure',
        children: [{
          tag: 'img',
          attrs: { src: api.getFileUrl(img.id, false) }
        }]
      }));

      const formData = new URLSearchParams();
      formData.append('access_token', 'c2a41fe0a6a592a1cffba1ab0f2f650ff0cf46a9964837ba09cba7df7fad');
      formData.append('title', album.name); // 默认标题 = 相册名
      formData.append('author_name', '媚黑资源库'); // 发布者不变
      formData.append('author_url', 'https://t.me/bangnihuijia'); // 发布链接不变
      formData.append('content', JSON.stringify(contentNodes));
      formData.append('return_content', 'false');

      const response = await fetch('https://api.telegra.ph/createPage', {
        method: 'POST',
        body: formData
      });
      const data = await response.json();

      if (data.ok) {
        const pageUrl = data.result.url;
        setTgResultUrl(pageUrl);
        setShowTgModal(true);
        toast.success('Telegra.ph 页面发布成功！');
      } else {
        toast.error('Telegra.ph 发布失败：' + data.error);
      }
    } catch (err: any) {
      toast.error('请求 Telegra.ph 网络异常：' + err.message);
    } finally {
      setTgPublishing(false);
    }
  };

  const handleConfirm = async () => {
    if (!confirm || !albumid) return;
    setBusy(true);
    try {
      if (confirm.kind === 'album') {
        await api.deleteAlbum(albumid);
        toast.success('相册已解散');
        navigate('/manage');
      } else {
        await api.deleteImagePhysically(confirm.image.id);
        toast.success('图片已在物理世界彻底抹除');
        setLightbox(null);
        setImages((prev) => prev.filter((img) => img.id !== confirm.image.id));
        setConfirm(null);
      }
    } catch (err) {
      toast.error((err as Error)?.message ?? '操作失败');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="album-detail-container">
      <PageHeader
        title={album?.name ?? '相册详情'}
        subtitle={
          album
            ? `${images.length} 张图片${album.has_password ? ' · 加密相册' : ''}`
            : undefined
        }
        onBack={() => navigate('/manage')}
      >
        {album && (
          <>
            <motion.button
              type="button"
              className={`ad-icon-btn ${selecting ? 'active' : ''}`}
              whileTap={{ scale: 0.88 }}
              onClick={() => {
                setSelecting((s) => !s);
                setSelectedIds([]);
              }}
              title="多选操作"
            >
              <SquareCheck size={17} strokeWidth={1.5} />
            </motion.button>

            {/* 上传图片按钮 */}
            <motion.button
              type="button"
              className="ad-icon-btn"
              whileTap={{ scale: 0.88 }}
              onClick={() => fileInputRef.current?.click()}
              disabled={uploading}
              title="在本相册直接上传图片"
            >
              <Upload size={17} strokeWidth={1.5} />
            </motion.button>
            <input
              ref={fileInputRef}
              type="file"
              multiple
              accept="image/*"
              style={{ display: 'none' }}
              onChange={handleDirectUploadFiles}
            />

            {/* 从图库挑选 */}
            <motion.button
              type="button"
              className="ad-icon-btn"
              whileTap={{ scale: 0.88 }}
              onClick={() => setShowPicker(true)}
              title="从现有图库挑选"
            >
              <ImagePlus size={17} strokeWidth={1.5} />
            </motion.button>

            {/* Telegra.ph 一键发布与即时预览 */}
            <motion.button
              type="button"
              className="ad-icon-btn"
              whileTap={{ scale: 0.88 }}
              onClick={handlePublishToTelegraph}
              disabled={tgPublishing}
              title="发布到 Telegra.ph 并生成即时预览"
            >
              <Globe size={17} strokeWidth={1.5} />
            </motion.button>

            <motion.button
              type="button"
              className="ad-icon-btn danger"
              whileTap={{ scale: 0.88 }}
              onClick={() => setConfirm({ kind: 'album' })}
              title="解散相册"
            >
              <Trash2 size={17} strokeWidth={1.5} />
            </motion.button>
          </>
        )}
      </PageHeader>

      {uploading && (
        <div style={{ padding: '12px', textAlign: 'center', background: 'var(--accent-soft)', borderRadius: '10px', marginBottom: '14px', fontSize: '13px', fontWeight: 600, color: 'var(--accent)' }}>
          {uploadStatus}
        </div>
      )}

      {loading ? (
        <Loader text="正在提取相册归档..." />
      ) : locked ? (
        <motion.form
          className="ad-unlock-card"
          onSubmit={handleUnlock}
          initial={{ opacity: 0, y: 26, scale: 0.97 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          transition={{ type: 'spring', stiffness: 300, damping: 26 }}
        >
          <div className="ad-unlock-icon">
            <KeyRound size={26} strokeWidth={1.5} />
          </div>
          <h3 className="ad-unlock-title">这是一份加密相册</h3>
          <p className="ad-unlock-desc">输入创建者设置的相册提取码即可打开归档</p>
          <div className="ad-unlock-field">
            <input
              ref={unlockRef}
              className="ad-unlock-input"
              type="password"
              placeholder="请输入提取码"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              maxLength={32}
              autoComplete="off"
            />
            <button className="ad-unlock-btn" type="submit" disabled={unlocking}>
              {unlocking && <span className="ad-spinner" />}
              校验并解锁
            </button>
          </div>
        </motion.form>
      ) : notFound ? (
        <EmptyState
          variant="album"
          title="相册不存在或无权访问"
          description="它可能已被解散，请返回列表确认后重试"
        />
      ) : album ? (
        <>
          <div className="ad-head">
            {album.description && <p className="ad-head-desc">{album.description}</p>}
            <div className="ad-chips">
              <span className="ad-chip">
                <Calendar size={12} strokeWidth={1.5} />
                {new Date(album.created_at).toLocaleDateString('zh-CN', {
                  year: 'numeric',
                  month: 'short',
                  day: 'numeric',
                })}{' '}
                创建
              </span>
              <span className="ad-chip">
                <Images size={12} strokeWidth={1.5} />
                {images.length} 张图片
              </span>
              {images.length > 0 && (
                <span className="ad-chip">
                  归档 {formatBytes(images.reduce((sum, img) => sum + img.file_size, 0))}
                </span>
              )}
            </div>
          </div>

          {images.length === 0 ? (
            <EmptyState
              variant="album"
              title="相册还是空的"
              description="点击上方上传按钮或挑选图库图片加入这个相册吧"
              actionText="直接上传图片"
              onAction={() => fileInputRef.current?.click()}
            />
          ) : (
            <div className="ad-grid">
              {images.map((img, i) => {
                const selected = selectedIds.includes(img.id);
                return (
                  <FadeInUp key={img.id} delay={Math.min(i * 0.04, 0.4)}>
                    <motion.div
                      className={`ad-item ${selecting ? 'selecting' : ''} ${selected ? 'selected' : ''}`}
                      whileHover={{ y: selecting ? -2 : -4 }}
                      whileTap={{ scale: 0.96 }}
                      onClick={() => handleItemClick(img)}
                    >
                      <img src={api.getFileUrl(img.id, true)} alt={img.file_name} loading="lazy" draggable={false} />
                      {selecting && (
                        <span className="ad-check">
                          {selected && <Check size={13} strokeWidth={2.5} />}
                        </span>
                      )}
                      <span className="ad-name">{img.file_name}</span>
                    </motion.div>
                  </FadeInUp>
                );
              })}
            </div>
          )}
        </>
      ) : null}

      {/* 批量操作条 */}
      <AnimatePresence>
        {selecting && selectedIds.length > 0 && (
          <motion.div
            className="batch-bar"
            initial={{ opacity: 0, y: 44 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 44 }}
            transition={{ type: 'spring', stiffness: 320, damping: 28 }}
          >
            <span className="batch-bar-count">已选 {selectedIds.length} 张</span>
            <button
              className="batch-bar-btn danger"
              onClick={handleRemoveSelected}
              disabled={removing}
            >
              {removing && <span className="ad-spinner" />}
              <Trash2 size={15} strokeWidth={1.5} />
              移出相册
            </button>
            <button className="batch-bar-btn" onClick={exitSelectMode}>
              <X size={15} strokeWidth={1.5} />
              取消选择
            </button>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Telegra.ph 发布与即时预览成功弹窗 */}
      <AnimatePresence>
        {showTgModal && (
          <div style={{ position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.5)', display: 'grid', placeItems: 'center', zIndex: 1000, padding: '16px' }}>
            <motion.div
              style={{ background: 'var(--bg-surface)', padding: '24px', borderRadius: '18px', maxWidth: '460px', width: '100%', boxShadow: '0 20px 50px rgba(0,0,0,0.3)', border: '1px solid var(--border-subtle)' }}
              initial={{ scale: 0.9, opacity: 0 }}
              animate={{ scale: 1, opacity: 1 }}
              exit={{ scale: 0.9, opacity: 0 }}
            >
              <h3 style={{ fontSize: '18px', fontWeight: 700, marginBottom: '12px', display: 'flex', alignItems: 'center', gap: '8px' }}>
                🎉 Telegra.ph 页面发布成功
              </h3>
              <p style={{ fontSize: '13px', color: 'var(--text-muted)', marginBottom: '14px' }}>
                已默认采用相册名「{album?.name}」作为标题，署名与跳转链接已自动带入。
              </p>
              
              <div style={{ marginBottom: '14px' }}>
                <label style={{ fontSize: '12px', fontWeight: 600, color: 'var(--text-muted)' }}>Telegra.ph 网页地址：</label>
                <input type="text" readOnly value={tgResultUrl} style={{ width: '100%', padding: '8px 10px', marginTop: '4px', borderRadius: '8px', border: '1px solid var(--border-subtle)', background: 'var(--bg-surface-hover)', fontSize: '13px' }} />
              </div>

              <div style={{ marginBottom: '20px' }}>
                <label style={{ fontSize: '12px', fontWeight: 600, color: 'var(--text-muted)' }}>Telegram 即时预览测试链接（点击生成缓存）：</label>
                <input type="text" readOnly value={`https://instantview.telegram.org/samples/telegra.ph/?url=${encodeURIComponent(tgResultUrl)}`} style={{ width: '100%', padding: '8px 10px', marginTop: '4px', borderRadius: '8px', border: '1px solid var(--border-subtle)', background: 'var(--bg-surface-hover)', fontSize: '13px' }} />
              </div>

              <div style={{ display: 'flex', gap: '10px', justifyContent: 'flex-end' }}>
                <button
                  className="modal-btn"
                  onClick={() => {
                    navigator.clipboard.writeText(`https://instantview.telegram.org/samples/telegra.ph/?url=${encodeURIComponent(tgResultUrl)}`);
                    toast.success('已复制即时预览测试链接！');
                  }}
                >
                  复制预览链接
                </button>
                <a
                  href={`https://instantview.telegram.org/samples/telegra.ph/?url=${encodeURIComponent(tgResultUrl)}`}
                  target="_blank"
                  rel="noreferrer"
                  className="modal-btn modal-btn-primary"
                  style={{ textDecoration: 'none', display: 'inline-flex', alignItems: 'center', gap: '6px' }}
                >
                  前往生成即时预览 <ExternalLink size={14} />
                </a>
                <button className="modal-btn" onClick={() => setShowTgModal(false)}>
                  关闭
                </button>
              </div>
            </motion.div>
          </div>
        )}
      </AnimatePresence>

      <ImageLightbox
        image={lightbox}
        onClose={() => setLightbox(null)}
        onRequestDelete={(img) => setConfirm({ kind: 'image', image: img })}
        onChanged={() => load(accessPwd || undefined)}
      />

      <ConfirmSheet
        open={confirm !== null}
        title={confirm?.kind === 'album' ? '解散这本相册？' : '永久删除这张图片？'}
        description={
          confirm?.kind === 'album'
            ? '相册内的图片不会被删除，只会解除与相册的归属关系，此操作不可撤销。'
            : '此操作将同步删除 Telegram 频道中的原始消息，且不可恢复。'
        }
        confirmText={confirm?.kind === 'album' ? '确认解散' : '永久删除'}
        tone="danger"
        loading={busy}
        onConfirm={handleConfirm}
        onClose={() => setConfirm(null)}
      />

      <ImagePickerModal
        open={showPicker}
        title="挑选图片加入相册"
        subtitle={album ? `加入到「${album.name}」` : undefined}
        excludeIds={images.map((img) => img.id)}
        confirmText="加入相册"
        submitting={adding}
        onClose={() => setShowPicker(false)}
        onConfirm={handleAddImages}
      />
    </div>
  );
};

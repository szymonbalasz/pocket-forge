/** FR-GEN-26 / OQ-GEN-13: a candidate of another size than the working image (or with no image),
 *  shown alone and fit to view: no split or flip. */
export function PreviewAlone({ url }: { url: string }) {
  return (
    <div className="edt-preview-alone" data-testid="edt-preview-alone">
      <img src={url} alt="Candidate preview" draggable={false} />
    </div>
  );
}

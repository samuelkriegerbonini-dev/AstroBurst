export default function TabSpinner() {
  return (
    <div className="flex items-center justify-center py-12">
      <div
        className="w-5 h-5 rounded-full animate-spin"
        style={{ border: "2px solid transparent", borderTopColor: "var(--ab-teal)", borderRightColor: "rgba(20,184,166,0.3)" }}
      />
    </div>
  );
}

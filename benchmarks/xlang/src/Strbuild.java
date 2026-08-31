public class Strbuild {
    public static void main(String[] a) {
        final int n = 2000000;
        long t0 = System.nanoTime();
        StringBuilder b = new StringBuilder();
        for (int i = 0; i < n; i++) b.append(i % 1000);
        System.out.println("CHECK " + b.length());
        System.out.println("MS " + (System.nanoTime() - t0) / 1000000);
    }
}

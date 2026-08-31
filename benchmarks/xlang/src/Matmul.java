public class Matmul {
    public static void main(String[] args) {
        final int n = 300;
        double[][] a = new double[n][n], b = new double[n][n], c = new double[n][n];
        for (int i = 0; i < n; i++)
            for (int j = 0; j < n; j++) {
                a[i][j] = ((i * j) % 100) * 0.01;
                b[i][j] = ((i + j) % 100) * 0.01;
            }
        long t0 = System.nanoTime();
        for (int i = 0; i < n; i++)
            for (int j = 0; j < n; j++) {
                double s = 0;
                for (int k = 0; k < n; k++) s += a[i][k] * b[k][j];
                c[i][j] = s;
            }
        double t = 0;
        for (int i = 0; i < n; i++) for (int j = 0; j < n; j++) t += c[i][j];
        System.out.println("CHECK " + Math.round(t));
        System.out.println("MS " + (System.nanoTime() - t0) / 1000000);
    }
}

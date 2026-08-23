% Sample script for the Run button. `octave --no-gui --persist` keeps the interpreter (and any
% plot windows) alive after it finishes, so the session stays inspectable.
printf("Hello from CleeCode\n");
printf("octave:  %s\n", version());
A = [1 2; 3 4];
printf("det([1 2; 3 4]) = %g\n", det(A));
printf("mean of 1..10   = %g\n", mean(1:10));

figure(1);
plot(A(1,:),A(2,:));
grid minor;

#ifndef SHARED_H
#define SHARED_H	1

#include "nec2c.h"

/*------------------------------------------------------------------------*/

/* common  /crnt/ */
extern crnt_t crnt;

/* common  /dataj/ */
/* straws: per-thread (NEC_TLS) so the matrix fill can run on several
 * threads; see cmset() in matrix.c. */
#define NEC_TLS _Thread_local
extern NEC_TLS dataj_t dataj;

/* common  /data/ */
extern data_t data;

/* pointers to input/output files */
extern FILE *input_fp, *output_fp, *plot_fp;

/* common  /fpat/ */
extern fpat_t fpat;

/*common  /ggrid/ */
extern ggrid_t ggrid;

/* common  /gnd/ */
extern gnd_t gnd;

/* common  /gwav/ */
extern NEC_TLS gwav_t gwav;

/* common  /incom/ */
extern NEC_TLS incom_t incom;

/* common  /matpar/ */
extern matpar_t matpar;

/* common  /netcx/ */
extern netcx_t netcx;

/* common  /plot/ */
extern plot_t plot;

/* common  /save/ */
extern save_t save;

/* common  /segj/ */
extern NEC_TLS segj_t segj;

/* common  /smat/ */
extern smat_t smat;

/* common  /tmi/ */
extern NEC_TLS tmi_t tmi;

/* common  /vsorc/ */
extern vsorc_t vsorc;

/* common  /yparm/ */
extern yparm_t yparm;

/* common  /zload/ */
extern zload_t zload;

/*------------------------------------------------------------------------*/

#endif
